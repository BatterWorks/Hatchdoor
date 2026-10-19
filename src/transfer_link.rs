//! Transfer links (ADR-27, widened by ADR-32): short-lived URLs, minted by an
//! authenticated MCP call, that carry their own credential for one attachment
//! download or one upload to one target, an attachment or a note. An agent inside an MCP client holds neither the MCP
//! token nor the server's address; a transfer link gives it both, for one file
//! and five minutes, without handing over the token.
//!
//! A link is stateless apart from one thing. Its query string carries an
//! expiry and a BLAKE3 keyed-hash signature over everything it grants, so the
//! server can check it without remembering it. The exception is an upload
//! link's single use, which needs a record of spent links; that record lives
//! here, in memory, and forgets each entry once the link would have expired
//! anyway. Beside it sits the one other thing remembered: the name of the MCP
//! client that asked for each upload link, so the upload is recorded as that
//! client's write (ADR-49). It is kept here and not in the link because the
//! link's URL and signature must not change, and it is forgotten on the same
//! schedule.
//!
//! The random master key is held only in memory and never written anywhere.
//! Each request signs or checks under a [`SigningKey`] derived from it, from
//! the MCP token's revision (`RuntimeConfig::value_revision`), and from the
//! token itself. The revision strands every link on any change to the token,
//! including a change back to an earlier value. The token closes the gap a
//! revision alone leaves: a tool call admitted on the old token that mints
//! after a rotation signs under that old token, which the redeeming request,
//! bound to the new one, no longer matches. A restart strands every link too,
//! because the master key is gone.
//!
//! What this module does not do: decide whether MCP or MCP write mode is on,
//! spend the tool quota, or read and write files. The HTTP adapter
//! (`handlers/transfer.rs`) re-reads the live configuration per request and
//! runs the transfer through the same read and write cores the bearer routes
//! use.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;

use crate::runtime_config::RuntimeConfig;
use crate::vault_read::encode_relative_path;
use crate::vault_registry::VaultId;

/// How long a link stays usable after it is minted. Fixed by ADR-27, with no
/// setting.
pub const LINK_LIFETIME: Duration = Duration::from_secs(5 * 60);

/// The query parameter that carries a link's signature. The request trace
/// span redacts it (`auth::redact_query_token`).
pub const SIGNATURE_PARAM: &str = "signature";

/// The query parameter a replacing note link carries its expected content
/// hash in, base64url-encoded.
const EXPECTED_HASH_PARAM: &str = "expected_content_hash";
const TOKEN_KEY: &str = "HATCHDOOR_MCP_BEARER_TOKEN";
const DOMAIN: &[u8] = b"hatchdoor transfer link v1";
const NONCE_BYTES: usize = 16;

/// The process's link-signing state. One instance lives in `AppState`.
pub struct TransferLinks {
    master_key: [u8; 32],
    /// Upload links already redeemed, by nonce, with the Unix second each one
    /// expires at. An entry is dropped once its link could no longer verify.
    spent_uploads: Mutex<HashMap<[u8; NONCE_BYTES], u64>>,
    /// The name of the client that asked for each upload link not yet
    /// redeemed, by nonce, with the Unix second the link expires at. Only a
    /// client that sent a name has an entry.
    upload_clients: Mutex<HashMap<[u8; NONCE_BYTES], (u64, String)>>,
}

/// A redeemed upload link: the rule it allows the upload under, and the name
/// of the client that asked for the link. `client_name` is `None` when that
/// client sent no name or the name is no longer remembered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedeemedLink<Rule> {
    pub rule: Rule,
    pub client_name: Option<String>,
}

impl<Rule> RedeemedLink<Rule> {
    /// The same redemption with its rule restated for the caller.
    pub fn map_rule<Other>(self, restate: impl FnOnce(Rule) -> Other) -> RedeemedLink<Other> {
        RedeemedLink {
            rule: restate(self.rule),
            client_name: self.client_name,
        }
    }
}

/// The key one request mints or checks links under, from
/// [`TransferLinks::key`]. Opaque, and never serialized.
pub struct SigningKey([u8; 32]);

/// What a link grants, beyond its Vault and path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Grant {
    Download,
    Upload {
        /// Whether the upload may replace a file already at the target.
        overwrite: bool,
        nonce: [u8; NONCE_BYTES],
    },
    /// An upload that replaces an existing note, but only while the note
    /// still has the content hash it had when the link was minted (ADR-32).
    /// A grant of its own rather than a field on `Upload`, so an attachment
    /// link signs and reads exactly as it did before notes could be uploaded.
    ReplaceNote {
        expected_content_hash: String,
        nonce: [u8; NONCE_BYTES],
    },
}

/// What a redeemed note upload link allows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoteUploadRule {
    /// Create the note; an existing note at the target is a conflict.
    Create,
    /// Replace the existing note if its content hash is still this one.
    Replace { expected_content_hash: String },
}

/// A freshly minted link: the URL to hand the agent and the Unix second it
/// stops working.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MintedLink {
    pub url: String,
    pub expires_at: u64,
}

/// Why a presented link was refused. Deliberately coarse: a caller learns
/// whether asking again could help, never which part of a forged link failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkRefusal {
    /// Missing, malformed, signed for something else, or signed under a key
    /// this process no longer holds.
    Invalid,
    Expired,
    /// An upload link that has already been redeemed once.
    Spent,
}

impl LinkRefusal {
    pub fn code(self) -> &'static str {
        match self {
            Self::Invalid => "transfer_link_invalid",
            Self::Expired => "transfer_link_expired",
            Self::Spent => "transfer_link_spent",
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            Self::Invalid => {
                "This transfer link is not valid for this request. Links stop working when the server restarts or its MCP token changes; ask for a new one."
            }
            Self::Expired => "This transfer link has expired; ask for a new one.",
            Self::Spent => {
                "This upload link has already been used. Each upload link works once; ask for a new one."
            }
        }
    }
}

impl Default for TransferLinks {
    fn default() -> Self {
        Self::new()
    }
}

impl TransferLinks {
    /// A fresh random key. Panics only if the operating system cannot supply
    /// randomness, in which case the process cannot mint tokens of any kind.
    pub fn new() -> Self {
        let mut master_key = [0_u8; 32];
        getrandom::fill(&mut master_key).expect("operating system randomness");
        Self {
            master_key,
            spent_uploads: Mutex::new(HashMap::new()),
            upload_clients: Mutex::new(HashMap::new()),
        }
    }

    /// The key a request works under. `mcp_token` is the token the request
    /// was admitted on: the one its bound configuration snapshot holds when
    /// minting, and the live one when redeeming.
    pub fn key(&self, runtime_config: &RuntimeConfig, mcp_token: &str) -> SigningKey {
        let mut derivation = blake3::Hasher::new_keyed(&self.master_key);
        derivation.update(DOMAIN);
        derivation.update(&runtime_config.value_revision(TOKEN_KEY).to_le_bytes());
        derivation.update(mcp_token.as_bytes());
        SigningKey(derivation.finalize().into())
    }

    /// A download link for one attachment, built on `base`, the absolute
    /// origin from `McpConfig::link_base`.
    pub fn mint_download(
        &self,
        key: &SigningKey,
        base: &str,
        vault_id: VaultId,
        relative_path: &str,
    ) -> MintedLink {
        mint(key, base, vault_id, relative_path, Grant::Download, now())
    }

    /// An upload link for one target path under one overwrite rule. For a
    /// note target this is the creating link only: a link that replaces a
    /// note is [`TransferLinks::mint_note_replace`]. `client_name` is the
    /// name of the MCP client asking, remembered until the link expires.
    pub fn mint_upload(
        &self,
        key: &SigningKey,
        base: &str,
        vault_id: VaultId,
        target_relative_path: &str,
        overwrite: bool,
        client_name: Option<&str>,
    ) -> MintedLink {
        let nonce = fresh_nonce();
        let link = mint(
            key,
            base,
            vault_id,
            target_relative_path,
            Grant::Upload { overwrite, nonce },
            now(),
        );
        self.remember_client(nonce, &link, client_name);
        link
    }

    /// An upload link that replaces the note at `target_relative_path`, signed
    /// with the content hash the note must still have when the upload lands.
    pub fn mint_note_replace(
        &self,
        key: &SigningKey,
        base: &str,
        vault_id: VaultId,
        target_relative_path: &str,
        expected_content_hash: &str,
        client_name: Option<&str>,
    ) -> MintedLink {
        let nonce = fresh_nonce();
        let link = mint(
            key,
            base,
            vault_id,
            target_relative_path,
            Grant::ReplaceNote {
                expected_content_hash: expected_content_hash.to_string(),
                nonce,
            },
            now(),
        );
        self.remember_client(nonce, &link, client_name);
        link
    }

    /// Remember who asked for an upload link, and forget everyone whose link
    /// has expired since.
    fn remember_client(
        &self,
        nonce: [u8; NONCE_BYTES],
        link: &MintedLink,
        client_name: Option<&str>,
    ) {
        let mut clients = self
            .upload_clients
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let minted_at = link.expires_at - LINK_LIFETIME.as_secs();
        clients.retain(|_, (expires_at, _)| *expires_at >= minted_at);
        if let Some(name) = client_name {
            clients.insert(nonce, (link.expires_at, name.to_string()));
        }
    }

    /// Check a presented download link against the Vault and path it is being
    /// redeemed for. Checked when the transfer starts, so a transfer that
    /// started in time completes.
    pub fn verify_download(
        &self,
        key: &SigningKey,
        vault_id: VaultId,
        relative_path: &str,
        query: Option<&str>,
    ) -> Result<(), LinkRefusal> {
        let presented = PresentedLink::parse(query).ok_or(LinkRefusal::Invalid)?;
        if presented.grant != Grant::Download {
            return Err(LinkRefusal::Invalid);
        }
        check(key, vault_id, relative_path, &presented, now())
    }

    /// Check a presented upload link and spend it. The rule returned is
    /// whether the link allows overwriting an existing file. A link is spent by its first
    /// redemption that gets this far, whether or not the upload that follows
    /// succeeds. A link that replaces a note is not an attachment link and
    /// is refused here.
    pub fn redeem_upload(
        &self,
        key: &SigningKey,
        vault_id: VaultId,
        target_relative_path: &str,
        query: Option<&str>,
    ) -> Result<RedeemedLink<bool>, LinkRefusal> {
        self.redeem_upload_at(key, vault_id, target_relative_path, query, now())
    }

    fn redeem_upload_at(
        &self,
        key: &SigningKey,
        vault_id: VaultId,
        target_relative_path: &str,
        query: Option<&str>,
        at: u64,
    ) -> Result<RedeemedLink<bool>, LinkRefusal> {
        let redeemed = self.redeem_any_upload(key, vault_id, target_relative_path, query, at)?;
        match redeemed.rule {
            Grant::Upload { overwrite, .. } => Ok(redeemed.map_rule(|_| overwrite)),
            _ => Err(LinkRefusal::Invalid),
        }
    }

    /// [`TransferLinks::redeem_upload`] for a note target: check the link,
    /// spend it, and say whether it creates the note or replaces it under an
    /// expected hash. A plain upload link that allows overwriting is refused,
    /// because no link replaces a note without its hash (ADR-32).
    pub fn redeem_note_upload(
        &self,
        key: &SigningKey,
        vault_id: VaultId,
        target_relative_path: &str,
        query: Option<&str>,
    ) -> Result<RedeemedLink<NoteUploadRule>, LinkRefusal> {
        let redeemed = self.redeem_any_upload(key, vault_id, target_relative_path, query, now())?;
        let rule = match redeemed.rule {
            Grant::Upload {
                overwrite: false, ..
            } => NoteUploadRule::Create,
            Grant::ReplaceNote {
                expected_content_hash,
                ..
            } => NoteUploadRule::Replace {
                expected_content_hash,
            },
            _ => return Err(LinkRefusal::Invalid),
        };
        Ok(RedeemedLink {
            rule,
            client_name: redeemed.client_name,
        })
    }

    /// Verify an upload link of either kind and spend it, returning its grant
    /// and forgetting the name of the client that asked for it.
    fn redeem_any_upload(
        &self,
        key: &SigningKey,
        vault_id: VaultId,
        target_relative_path: &str,
        query: Option<&str>,
        at: u64,
    ) -> Result<RedeemedLink<Grant>, LinkRefusal> {
        let presented = PresentedLink::parse(query).ok_or(LinkRefusal::Invalid)?;
        let nonce = match &presented.grant {
            Grant::Upload { nonce, .. } | Grant::ReplaceNote { nonce, .. } => *nonce,
            Grant::Download => return Err(LinkRefusal::Invalid),
        };
        check(key, vault_id, target_relative_path, &presented, at)?;

        let mut spent = self
            .spent_uploads
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        spent.retain(|_, expires_at| *expires_at >= at);
        if spent.insert(nonce, presented.expires_at).is_some() {
            return Err(LinkRefusal::Spent);
        }
        drop(spent);

        let mut clients = self
            .upload_clients
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let client_name = clients.remove(&nonce).map(|(_, name)| name);
        clients.retain(|_, (expires_at, _)| *expires_at >= at);
        Ok(RedeemedLink {
            rule: presented.grant,
            client_name,
        })
    }
}

/// A random upload nonce, the one thing that makes each upload link single
/// use. Panics only where [`TransferLinks::new`] would.
fn fresh_nonce() -> [u8; NONCE_BYTES] {
    let mut nonce = [0_u8; NONCE_BYTES];
    getrandom::fill(&mut nonce).expect("operating system randomness");
    nonce
}

/// The spelling of `relative_path` a link signs and carries: trimmed, with
/// `.` and empty segments dropped. The Vault reads and writes ignore them
/// too, but a client removes `.` segments before sending (RFC 3986 5.2.4), so
/// a link that kept one would be redeemed at a path it was not signed for.
/// `..` is left alone: the tools refuse it before any link is minted.
pub fn link_path(relative_path: &str) -> String {
    relative_path
        .trim()
        .split('/')
        .filter(|segment| !matches!(*segment, "" | "."))
        .collect::<Vec<_>>()
        .join("/")
}

fn mint(
    key: &SigningKey,
    base: &str,
    vault_id: VaultId,
    relative_path: &str,
    grant: Grant,
    issued_at: u64,
) -> MintedLink {
    let relative_path = &link_path(relative_path);
    let expires_at = issued_at + LINK_LIFETIME.as_secs();
    let signature = sign(key, vault_id, relative_path, &grant, expires_at);
    let mut query = format!("expires={expires_at}");
    match &grant {
        Grant::Download => {}
        Grant::Upload { overwrite, nonce } => {
            query.push_str(&format!("&overwrite={overwrite}&nonce={}", encode(nonce)));
        }
        Grant::ReplaceNote {
            expected_content_hash,
            nonce,
        } => {
            query.push_str(&format!(
                "&overwrite=true&nonce={}&{EXPECTED_HASH_PARAM}={}",
                encode(nonce),
                encode(expected_content_hash.as_bytes())
            ));
        }
    }
    query.push_str(&format!("&{SIGNATURE_PARAM}={}", encode(&signature)));
    MintedLink {
        url: format!(
            "{base}/api/v1/vaults/{vault_id}/transfers/{}?{query}",
            encode_relative_path(relative_path)
        ),
        expires_at,
    }
}

fn check(
    key: &SigningKey,
    vault_id: VaultId,
    relative_path: &str,
    presented: &PresentedLink,
    at: u64,
) -> Result<(), LinkRefusal> {
    let expected = sign(
        key,
        vault_id,
        relative_path,
        &presented.grant,
        presented.expires_at,
    );
    // Signature first: an expiry is only worth reporting on a link this
    // server actually issued.
    if !crate::auth::constant_time_eq(&expected, &presented.signature) {
        return Err(LinkRefusal::Invalid);
    }
    if at > presented.expires_at {
        return Err(LinkRefusal::Expired);
    }
    Ok(())
}

fn sign(
    key: &SigningKey,
    vault_id: VaultId,
    relative_path: &str,
    grant: &Grant,
    expires_at: u64,
) -> [u8; 32] {
    let mut message = blake3::Hasher::new_keyed(&key.0);
    let vault_id = vault_id.to_string();
    for field in [vault_id.as_bytes(), relative_path.as_bytes()] {
        message.update(&(field.len() as u64).to_le_bytes());
        message.update(field);
    }
    message.update(&expires_at.to_le_bytes());
    match grant {
        Grant::Download => {
            message.update(b"download");
        }
        Grant::Upload { overwrite, nonce } => {
            message.update(b"upload");
            message.update(&[u8::from(*overwrite)]);
            message.update(nonce);
        }
        Grant::ReplaceNote {
            expected_content_hash,
            nonce,
        } => {
            message.update(b"replace note");
            message.update(nonce);
            message.update(&(expected_content_hash.len() as u64).to_le_bytes());
            message.update(expected_content_hash.as_bytes());
        }
    }
    message.finalize().into()
}

/// The fields a redemption request presents in its query string.
struct PresentedLink {
    expires_at: u64,
    grant: Grant,
    signature: Vec<u8>,
}

impl PresentedLink {
    fn parse(query: Option<&str>) -> Option<Self> {
        let mut expires_at = None;
        let mut overwrite = None;
        let mut nonce = None;
        let mut expected_content_hash = None;
        let mut signature = None;
        for pair in query?.split('&') {
            let (key, value) = pair.split_once('=')?;
            let slot = match key {
                "expires" => &mut expires_at,
                "overwrite" => &mut overwrite,
                "nonce" => &mut nonce,
                EXPECTED_HASH_PARAM => &mut expected_content_hash,
                SIGNATURE_PARAM => &mut signature,
                _ => continue,
            };
            // A repeated parameter is ambiguous about which value was signed.
            if slot.replace(value).is_some() {
                return None;
            }
        }
        let grant = match (overwrite, nonce, expected_content_hash) {
            (None, None, None) => Grant::Download,
            (Some(overwrite), Some(nonce), None) => Grant::Upload {
                overwrite: match overwrite {
                    "true" => true,
                    "false" => false,
                    _ => return None,
                },
                nonce: decode(nonce)?.try_into().ok()?,
            },
            // A replacing note link always says it overwrites; any other
            // spelling is not a link this server minted.
            (Some("true"), Some(nonce), Some(hash)) => Grant::ReplaceNote {
                expected_content_hash: String::from_utf8(decode(hash)?).ok()?,
                nonce: decode(nonce)?.try_into().ok()?,
            },
            _ => return None,
        };
        Some(Self {
            expires_at: expires_at?.parse().ok()?,
            grant,
            signature: decode(signature?)?,
        })
    }
}

fn encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn decode(text: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(text)
        .ok()
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    const BASE: &str = "http://127.0.0.1:42824";
    const TOKEN: &str = "mcp-token";

    fn vault() -> VaultId {
        VaultId::from_str("00000000-0000-4000-8000-000000000001").expect("vault id")
    }

    fn other_vault() -> VaultId {
        VaultId::from_str("00000000-0000-4000-8000-000000000002").expect("vault id")
    }

    fn query_of(link: &MintedLink) -> String {
        link.url.split_once('?').expect("query").1.to_string()
    }

    fn path_of(link: &MintedLink) -> String {
        link.url.split_once('?').expect("query").0.to_string()
    }

    fn upload_grant(nonce: u8) -> Grant {
        Grant::Upload {
            overwrite: false,
            nonce: [nonce; NONCE_BYTES],
        }
    }

    #[test]
    fn a_download_link_verifies_for_its_own_vault_and_path_only() {
        let config = RuntimeConfig::for_tests();
        let links = TransferLinks::new();
        let key = links.key(&config, TOKEN);
        let link = links.mint_download(&key, BASE, vault(), "files/manual.pdf");
        let query = query_of(&link);

        assert_eq!(
            links.verify_download(&key, vault(), "files/manual.pdf", Some(&query)),
            Ok(())
        );
        // Any number of times until it expires.
        assert_eq!(
            links.verify_download(&key, vault(), "files/manual.pdf", Some(&query)),
            Ok(())
        );
        assert_eq!(
            links.verify_download(&key, vault(), "files/other.pdf", Some(&query)),
            Err(LinkRefusal::Invalid)
        );
        assert_eq!(
            links.verify_download(&key, other_vault(), "files/manual.pdf", Some(&query)),
            Err(LinkRefusal::Invalid)
        );
        assert_eq!(
            links.verify_download(&key, vault(), "files/manual.pdf", None),
            Err(LinkRefusal::Invalid)
        );
    }

    #[test]
    fn a_link_is_absolute_on_the_transfer_route_with_the_asset_encoding() {
        let config = RuntimeConfig::for_tests();
        let links = TransferLinks::new();
        let before = now();
        let link = links.mint_download(
            &links.key(&config, TOKEN),
            "https://notes.example.com",
            vault(),
            "files/AEG manual (FR).pdf",
        );
        assert_eq!(
            path_of(&link),
            format!(
                "https://notes.example.com/api/v1/vaults/{}/transfers/files/AEG%20manual%20%28FR%29.pdf",
                vault()
            )
        );
        let lifetime = LINK_LIFETIME.as_secs();
        assert!(
            (before + lifetime..=now() + lifetime).contains(&link.expires_at),
            "a link lives five minutes"
        );
    }

    /// curl and browsers remove `.` segments before sending (RFC 3986 5.2.4),
    /// so a link must not carry one: the request would name a path the
    /// signature was not made over (#586).
    #[test]
    fn a_link_drops_dot_and_empty_segments_so_a_client_requests_the_signed_path() {
        let config = RuntimeConfig::for_tests();
        let links = TransferLinks::new();
        let key = links.key(&config, TOKEN);
        let route = format!("{BASE}/api/v1/vaults/{}/transfers/", vault());

        for spelling in [
            "./Media/a shot.png",
            "Media/./a shot.png",
            "Media//a shot.png",
        ] {
            let link = links.mint_download(&key, BASE, vault(), spelling);
            assert_eq!(path_of(&link), format!("{route}Media/a%20shot.png"));
            assert_eq!(
                links.verify_download(&key, vault(), "Media/a shot.png", Some(&query_of(&link))),
                Ok(())
            );
        }

        let upload = links.mint_upload(&key, BASE, vault(), "./in/scan.png", false, None);
        assert_eq!(path_of(&upload), format!("{route}in/scan.png"));
        assert_eq!(
            links
                .redeem_upload(&key, vault(), "in/scan.png", Some(&query_of(&upload)))
                .map(|redeemed| redeemed.rule),
            Ok(false)
        );

        let replace = links.mint_note_replace(&key, BASE, vault(), "./Notes/a.md", "abc123", None);
        assert_eq!(path_of(&replace), format!("{route}Notes/a.md"));
        assert!(
            links
                .redeem_note_upload(&key, vault(), "Notes/a.md", Some(&query_of(&replace)))
                .is_ok()
        );
    }

    #[test]
    fn a_link_is_refused_after_it_expires_and_a_tampered_expiry_is_invalid() {
        let config = RuntimeConfig::for_tests();
        let links = TransferLinks::new();
        let key = links.key(&config, TOKEN);
        let issued = now() - LINK_LIFETIME.as_secs() - 1;
        let link = mint(&key, BASE, vault(), "a.pdf", Grant::Download, issued);
        let query = query_of(&link);
        assert_eq!(
            links.verify_download(&key, vault(), "a.pdf", Some(&query)),
            Err(LinkRefusal::Expired)
        );

        let extended = query.replace(
            &format!("expires={}", link.expires_at),
            &format!("expires={}", link.expires_at + 3600),
        );
        assert_eq!(
            links.verify_download(&key, vault(), "a.pdf", Some(&extended)),
            Err(LinkRefusal::Invalid)
        );
    }

    #[test]
    fn an_upload_link_works_once_for_its_own_target() {
        let config = RuntimeConfig::for_tests();
        let links = TransferLinks::new();
        let key = links.key(&config, TOKEN);
        let link = links.mint_upload(&key, BASE, vault(), "in/scan.png", true, None);
        let query = query_of(&link);

        assert_eq!(
            links
                .redeem_upload(&key, vault(), "in/other.png", Some(&query))
                .map(|redeemed| redeemed.rule),
            Err(LinkRefusal::Invalid)
        );
        assert_eq!(
            links
                .redeem_upload(&key, vault(), "in/scan.png", Some(&query))
                .map(|redeemed| redeemed.rule),
            Ok(true)
        );
        assert_eq!(
            links
                .redeem_upload(&key, vault(), "in/scan.png", Some(&query))
                .map(|redeemed| redeemed.rule),
            Err(LinkRefusal::Spent)
        );
    }

    #[test]
    fn an_upload_link_cannot_have_its_overwrite_rule_flipped_or_download() {
        let config = RuntimeConfig::for_tests();
        let links = TransferLinks::new();
        let key = links.key(&config, TOKEN);
        let link = links.mint_upload(&key, BASE, vault(), "in/scan.png", false, None);
        let query = query_of(&link);
        let flipped = query.replace("overwrite=false", "overwrite=true");
        assert_eq!(
            links
                .redeem_upload(&key, vault(), "in/scan.png", Some(&flipped))
                .map(|redeemed| redeemed.rule),
            Err(LinkRefusal::Invalid)
        );
        assert_eq!(
            links.verify_download(&key, vault(), "in/scan.png", Some(&query)),
            Err(LinkRefusal::Invalid)
        );

        let download = links.mint_download(&key, BASE, vault(), "in/scan.png");
        assert_eq!(
            links
                .redeem_upload(&key, vault(), "in/scan.png", Some(&query_of(&download)))
                .map(|redeemed| redeemed.rule),
            Err(LinkRefusal::Invalid),
            "a download link is not an upload link"
        );
    }

    #[test]
    fn an_expired_upload_link_is_refused_and_not_spent() {
        let config = RuntimeConfig::for_tests();
        let links = TransferLinks::new();
        let key = links.key(&config, TOKEN);
        let link = mint(&key, BASE, vault(), "in/scan.png", upload_grant(7), 1_000);
        let query = query_of(&link);
        let late = 1_000 + LINK_LIFETIME.as_secs() + 1;
        assert_eq!(
            links
                .redeem_upload_at(&key, vault(), "in/scan.png", Some(&query), late)
                .map(|redeemed| redeemed.rule),
            Err(LinkRefusal::Expired)
        );
        assert!(links.spent_uploads.lock().expect("lock").is_empty());
    }

    #[test]
    fn spent_uploads_are_forgotten_once_they_could_no_longer_verify() {
        let config = RuntimeConfig::for_tests();
        let links = TransferLinks::new();
        let key = links.key(&config, TOKEN);
        let link = mint(&key, BASE, vault(), "a.png", upload_grant(1), 1_000);
        links
            .redeem_upload_at(&key, vault(), "a.png", Some(&query_of(&link)), 1_000)
            .expect("first use");
        let later = mint(&key, BASE, vault(), "b.png", upload_grant(2), 2_000);
        links
            .redeem_upload_at(&key, vault(), "b.png", Some(&query_of(&later)), 2_000)
            .expect("second link");
        let spent = links.spent_uploads.lock().expect("lock");
        assert_eq!(spent.len(), 1, "the first link's record was pruned");
    }

    #[test]
    fn every_link_dies_when_the_mcp_token_changes_even_back_to_its_old_value() {
        let config = RuntimeConfig::for_tests();
        let links = TransferLinks::new();
        let set_token = |token: &str| {
            config
                .save([(TOKEN_KEY.to_string(), token.to_string())])
                .expect("save token");
        };
        set_token("token-a");
        let key = links.key(&config, "token-a");
        let download = links.mint_download(&key, BASE, vault(), "a.pdf");
        let upload = links.mint_upload(&key, BASE, vault(), "b.pdf", false, None);

        set_token("token-b");
        set_token("token-a");
        let key = links.key(&config, "token-a");
        assert_eq!(
            links.verify_download(&key, vault(), "a.pdf", Some(&query_of(&download))),
            Err(LinkRefusal::Invalid)
        );
        assert_eq!(
            links
                .redeem_upload(&key, vault(), "b.pdf", Some(&query_of(&upload)))
                .map(|redeemed| redeemed.rule),
            Err(LinkRefusal::Invalid)
        );
    }

    /// A tool call admitted on the old token that mints after the rotation
    /// has published reads the new revision, but still holds the old token.
    #[test]
    fn a_link_minted_on_the_old_token_after_a_rotation_does_not_survive_it() {
        let config = RuntimeConfig::for_tests();
        let links = TransferLinks::new();
        config
            .save([(TOKEN_KEY.to_string(), "rotated".to_string())])
            .expect("rotate");
        let straggler =
            links.mint_download(&links.key(&config, "original"), BASE, vault(), "a.pdf");
        assert_eq!(
            links.verify_download(
                &links.key(&config, "rotated"),
                vault(),
                "a.pdf",
                Some(&query_of(&straggler))
            ),
            Err(LinkRefusal::Invalid)
        );
    }

    #[test]
    fn a_restart_strands_every_link() {
        let config = RuntimeConfig::for_tests();
        let before = TransferLinks::new();
        let link = before.mint_download(&before.key(&config, TOKEN), BASE, vault(), "a.pdf");
        let after = TransferLinks::new();
        assert_eq!(
            after.verify_download(
                &after.key(&config, TOKEN),
                vault(),
                "a.pdf",
                Some(&query_of(&link))
            ),
            Err(LinkRefusal::Invalid)
        );
    }

    #[test]
    fn a_note_link_creates_or_replaces_under_its_signed_hash_once() {
        let config = RuntimeConfig::for_tests();
        let links = TransferLinks::new();
        let key = links.key(&config, TOKEN);

        let create = links.mint_upload(&key, BASE, vault(), "Imports/Report.md", false, None);
        assert_eq!(
            links
                .redeem_note_upload(&key, vault(), "Imports/Report.md", Some(&query_of(&create)))
                .map(|redeemed| redeemed.rule),
            Ok(NoteUploadRule::Create)
        );

        let replace =
            links.mint_note_replace(&key, BASE, vault(), "Imports/Report.md", "abc123", None);
        let query = query_of(&replace);
        assert!(query.contains("overwrite=true"), "{query}");
        assert_eq!(
            links
                .redeem_note_upload(&key, vault(), "Imports/Other.md", Some(&query))
                .map(|redeemed| redeemed.rule),
            Err(LinkRefusal::Invalid)
        );
        assert_eq!(
            links
                .redeem_note_upload(&key, vault(), "Imports/Report.md", Some(&query))
                .map(|redeemed| redeemed.rule),
            Ok(NoteUploadRule::Replace {
                expected_content_hash: "abc123".to_string()
            })
        );
        assert_eq!(
            links
                .redeem_note_upload(&key, vault(), "Imports/Report.md", Some(&query))
                .map(|redeemed| redeemed.rule),
            Err(LinkRefusal::Spent)
        );
    }

    #[test]
    fn a_replacing_note_link_cannot_lose_or_change_its_hash_or_overwrite_rule() {
        let config = RuntimeConfig::for_tests();
        let links = TransferLinks::new();
        let key = links.key(&config, TOKEN);
        let link = links.mint_note_replace(&key, BASE, vault(), "a.md", "abc123", None);
        let query = query_of(&link);
        let hash_param = format!("&expected_content_hash={}", encode(b"abc123"));
        assert!(query.contains(&hash_param), "{query}");

        for tampered in [
            query.replace(
                &hash_param,
                &format!("&expected_content_hash={}", encode(b"other")),
            ),
            query.replace(&hash_param, ""),
            query.replace("overwrite=true", "overwrite=false"),
            format!("{query}{hash_param}"),
        ] {
            assert_eq!(
                links
                    .redeem_note_upload(&key, vault(), "a.md", Some(&tampered))
                    .map(|redeemed| redeemed.rule),
                Err(LinkRefusal::Invalid),
                "{tampered}"
            );
        }
        assert_eq!(
            links
                .redeem_upload(&key, vault(), "a.md", Some(&query))
                .map(|redeemed| redeemed.rule),
            Err(LinkRefusal::Invalid),
            "a note-replacing link is not an attachment link"
        );
        assert_eq!(
            links.verify_download(&key, vault(), "a.md", Some(&query)),
            Err(LinkRefusal::Invalid)
        );
    }

    #[test]
    fn a_plain_overwriting_link_never_replaces_a_note() {
        let config = RuntimeConfig::for_tests();
        let links = TransferLinks::new();
        let key = links.key(&config, TOKEN);
        let link = links.mint_upload(&key, BASE, vault(), "a.md", true, None);
        assert_eq!(
            links
                .redeem_note_upload(&key, vault(), "a.md", Some(&query_of(&link)))
                .map(|redeemed| redeemed.rule),
            Err(LinkRefusal::Invalid)
        );
    }

    #[test]
    fn a_repeated_or_malformed_parameter_is_invalid() {
        let config = RuntimeConfig::for_tests();
        let links = TransferLinks::new();
        let key = links.key(&config, TOKEN);
        let link = links.mint_download(&key, BASE, vault(), "a.pdf");
        let query = query_of(&link);
        for bad in [
            format!("{query}&expires=1"),
            query.replace("expires=", "expires=x"),
            format!("{query}&overwrite=true"),
            query.replace("signature=", "signature=%%"),
        ] {
            assert_eq!(
                links.verify_download(&key, vault(), "a.pdf", Some(&bad)),
                Err(LinkRefusal::Invalid),
                "{bad}"
            );
        }
    }

    #[test]
    fn an_upload_link_remembers_who_asked_for_it_without_changing_its_url() {
        let links = TransferLinks::new();
        let key = links.key(&RuntimeConfig::for_tests(), "token");
        let named = links.mint_upload(
            &key,
            BASE,
            vault(),
            "in/scan.png",
            false,
            Some("claude-code"),
        );
        let unnamed = links.mint_upload(&key, BASE, vault(), "in/scan.png", false, None);
        let replace =
            links.mint_note_replace(&key, BASE, vault(), "a.md", "abc123", Some("claude-code"));

        let parameters = |link: &MintedLink| -> Vec<String> {
            query_of(link)
                .split('&')
                .map(|pair| {
                    pair.split_once('=')
                        .expect("a key and a value")
                        .0
                        .to_string()
                })
                .collect()
        };
        assert_eq!(parameters(&named), parameters(&unnamed));
        assert_eq!(
            parameters(&named),
            ["expires", "overwrite", "nonce", "signature"]
        );
        assert!(!named.url.contains("claude-code") && !replace.url.contains("claude-code"));

        let redeemed = links
            .redeem_upload(&key, vault(), "in/scan.png", Some(&query_of(&named)))
            .expect("redeem the named link");
        assert_eq!(redeemed.client_name.as_deref(), Some("claude-code"));
        let redeemed = links
            .redeem_upload(&key, vault(), "in/scan.png", Some(&query_of(&unnamed)))
            .expect("redeem the unnamed link");
        assert_eq!(redeemed.client_name, None);
        let redeemed = links
            .redeem_note_upload(&key, vault(), "a.md", Some(&query_of(&replace)))
            .expect("redeem the note link");
        assert_eq!(redeemed.client_name.as_deref(), Some("claude-code"));
    }

    #[test]
    fn the_client_behind_an_upload_link_is_forgotten_when_the_link_expires() {
        let links = TransferLinks::new();
        let key = links.key(&RuntimeConfig::for_tests(), "token");
        let remembered = || links.upload_clients.lock().expect("clients").len();

        let stale = mint(&key, BASE, vault(), "a.png", upload_grant(1), 1_000);
        links.remember_client([1; NONCE_BYTES], &stale, Some("claude-code"));
        assert_eq!(remembered(), 1);

        let later = mint(&key, BASE, vault(), "b.png", upload_grant(2), 2_000);
        links.remember_client([2; NONCE_BYTES], &later, Some("codex"));
        assert_eq!(remembered(), 1, "minting drops the expired link's client");

        links
            .redeem_upload_at(&key, vault(), "b.png", Some(&query_of(&later)), 2_000)
            .expect("redeem");
        assert_eq!(
            remembered(),
            0,
            "and redeeming forgets the client it returns"
        );
    }
}
