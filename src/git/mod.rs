pub mod commit_cooldown;
pub mod config;
pub mod managed_checkout;
pub mod managed_sync;
pub mod managed_task;
pub mod message;
pub mod note_history;
pub mod sync;

use crate::vault_registry::{VaultGitMode, VaultSource};

pub use commit_cooldown::{
    COMMIT_COOLDOWN_TICK_INTERVAL, CommitCooldown, spawn_commit_cooldown_tick,
};
pub use config::{GitConfig, GitMode};
pub use managed_checkout::{
    ManagedCheckout, ManagedCheckoutError, ManagedCheckoutLease, ManagedCheckoutRequest,
    ManagedHttpsCredentials,
};
pub use managed_sync::{
    ManagedSyncConfig, ManagedSyncError, ManagedSyncMode, ManagedSyncOutcome, RecoveryPublication,
    synchronize_managed_checkout,
};
pub use managed_task::{
    CONFLICT_CODE, DEFAULT_POLL_INTERVAL, DEFAULT_TICK_INTERVAL, GitPollingClock,
    ManagedGitOutcome, ManagedGitScheduler, ManagedGitTurnConfig, RecoveryFailure, RecoveryResult,
    run_existing_git_commit_turn, run_existing_git_recovery_turn, run_existing_git_remote_turn,
    run_managed_git_commit_turn, run_managed_git_turn, run_managed_recovery_turn,
    spawn_scheduler_tick,
};
pub use message::{PendingBatch, WriteLedger, WriteOrigin, WriteRecord, build_commit_message};
pub use note_history::{FirstAdd, FirstAdds, HistoryRead, NoteHistory};
pub use sync::{
    CommitOutcome, GitError, commit_local, has_uncommitted_changes, init_local_repo,
    run_local_history_git_turn, validate_local_repo, validate_repo,
};

/// How long libgit2 may wait to open a connection to a remote before the
/// operation fails.
#[cfg(not(test))]
const NETWORK_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);
/// How long an open connection to a remote may go without moving a byte in
/// either direction before the operation fails. This is the stall watchdog:
/// it measures silence, not total duration, so a large but moving transfer
/// is never cut short, while a remote that stops answering mid-transfer
/// (a dropped VPN, a firewall that drops rather than rejects, a hung proxy)
/// releases the Vault's mutation lock and the one work lane within this
/// bound instead of holding both forever (#322).
///
/// Shutdown waits for an in-flight turn, so `stop_grace_period` in
/// `docker-compose.yml` must stay above this plus the connect bound.
#[cfg(not(test))]
const NETWORK_TRANSFER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);
// Test builds use short bounds so a stalled-remote test finishes quickly.
// Every other test talks to a local path remote, which opens no socket and
// never sees either value.
#[cfg(test)]
const NETWORK_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);
#[cfg(test)]
const NETWORK_TRANSFER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Give libgit2's sockets the bounds above, once per process, before any
/// fetch, push, or clone opens one.
///
/// The vendored libgit2 starts with both timeouts disabled, which means a
/// read on a connection the remote has stopped answering blocks forever. A
/// timeout surfaces as an ordinary libgit2 network error, which the callers
/// already classify as a retryable remote failure. Called from every call
/// site that opens a connection rather than from startup, so no entry point,
/// test or production, can reach the network with the bounds unset.
pub(crate) fn bound_network_waits() {
    static CONFIGURED: std::sync::Once = std::sync::Once::new();
    CONFIGURED.call_once(|| {
        let milliseconds = |duration: std::time::Duration| {
            libc::c_int::try_from(duration.as_millis()).unwrap_or(libc::c_int::MAX)
        };
        // SAFETY: these options are process-global integers that libgit2
        // reads when it opens a socket stream. `Once` makes this the only
        // write, and it happens before the caller opens its own connection.
        // Neither setter can fail; the `Result` only mirrors the options API.
        unsafe {
            let _ = git2::opts::set_server_connect_timeout_in_milliseconds(milliseconds(
                NETWORK_CONNECT_TIMEOUT,
            ));
            let _ = git2::opts::set_server_timeout_in_milliseconds(milliseconds(
                NETWORK_TRANSFER_TIMEOUT,
            ));
        }
    });
}

/// What a remote operation's credential callback did, read back once the
/// operation has failed to tell a refused sign-in from any other failure.
pub(crate) struct CredentialOffers {
    offered: std::rc::Rc<std::cell::Cell<bool>>,
}

impl CredentialOffers {
    /// Whether `error` means the remote refused the sign-in. libgit2 replays a
    /// refused credential up to its redirect-and-replay limit and then fails
    /// with a generic error, never `GIT_EAUTH`, so the callback refuses to
    /// offer the same credential twice, and that refusal comes back here as
    /// `ErrorCode::Auth`. A 403 after the credential went out is a refusal too:
    /// libgit2 reports it only as an unexpected status, with no status code
    /// of its own, so the vendored libgit2's exact wording (1.9,
    /// `transports/http.c`) is matched; a test with a real 403 pins it (#638).
    fn rejected(&self, error: &git2::Error) -> bool {
        self.offered.get()
            && error.class() == git2::ErrorClass::Http
            && error.message() == "unexpected http status code: 403"
    }
}

/// Whether a failed clone, fetch or push means the remote refused the
/// sign-in: `ErrorCode::Auth` (which the credential callback's refusal to
/// offer a token twice produces), or a 403 after the token went out.
pub(crate) fn sign_in_rejected(error: &git2::Error, offers: Option<&CredentialOffers>) -> bool {
    error.code() == git2::ErrorCode::Auth || offers.is_some_and(|offers| offers.rejected(error))
}

/// The status message for files a commit left out because their names are
/// not valid UTF-8, shared by managed sync and Local history (#638).
pub(crate) fn left_out_message(files: &[String]) -> String {
    format!(
        "these files were left out because their names are not valid UTF-8: {}",
        files.join(", ")
    )
}

/// Offer `username`/`token` to the configured remote only, and at most once
/// per URL (#638).
///
/// libgit2 asks again after every 401, and follows a redirect on a remote's
/// first answer to whatever host it names, asking that host for credentials
/// too. Returning the stored token every time sent it on to the redirect's
/// host and made one rejected token into a dozen failed logins per attempt.
/// The callback compares the scheme, host and port libgit2 is about to send to
/// with `repository_url`'s and refuses anything else, so the token never
/// leaves the configured remote and never goes over plain HTTP from HTTPS.
pub(crate) fn offer_credentials_once(
    callbacks: &mut git2::RemoteCallbacks<'_>,
    repository_url: &str,
    username: String,
    token: String,
) -> CredentialOffers {
    let configured = url_origin(repository_url);
    let offered = std::rc::Rc::new(std::cell::Cell::new(false));
    let record = std::rc::Rc::clone(&offered);
    let mut offered_to = Vec::<String>::new();
    callbacks.credentials(move |url, _username, allowed| {
        let refuse = |message: &str| {
            Err(git2::Error::new(
                git2::ErrorCode::Auth,
                git2::ErrorClass::Http,
                message,
            ))
        };
        if !configured
            .as_ref()
            .is_some_and(|configured| callback_origin_matches(url, configured))
        {
            return refuse("credentials are offered only to the configured remote");
        }
        if !allowed.contains(git2::CredentialType::USER_PASS_PLAINTEXT) {
            return refuse("the remote asked for a sign-in Hatchdoor does not offer");
        }
        if offered_to.iter().any(|earlier| earlier == url) {
            return refuse("the remote rejected the credentials");
        }
        offered_to.push(url.to_owned());
        record.set(true);
        git2::Cred::userpass_plaintext(&username, &token)
    });
    CredentialOffers { offered }
}

/// Whether the URL libgit2 hands the credential callback has `configured`'s
/// origin. libgit2 rebuilds that URL itself, writing an IPv6 host without its
/// brackets and leaving out a default port, so `https://[fd00::5]/x.git`
/// arrives as `https://fd00::5/x.git`, which [`url_origin`] would read as host
/// `fd00:` and port 5. An authority holding more than one colon is therefore
/// also read as a whole bracketless IPv6 host on the default port.
fn callback_origin_matches(url: &str, configured: &(String, String, u16)) -> bool {
    if url_origin(url).as_ref() == Some(configured) {
        return true;
    }
    let Some((scheme, rest)) = url.split_once("://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let authority = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    !authority.starts_with('[')
        && authority.matches(':').count() > 1
        && url_origin(&format!("{scheme}://[{authority}]/")).as_ref() == Some(configured)
}

/// A URL's scheme, host and port, lowercased, with the scheme's default port
/// filled in, so `https://Host/` and `https://host:443/x` compare equal. Any
/// user information is dropped. `None` for a URL with no recognisable origin.
fn url_origin(url: &str) -> Option<(String, String, u16)> {
    let (scheme, rest) = url.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let (host, port) = match authority.strip_prefix('[') {
        Some(bracketed) => {
            let (host, suffix) = bracketed.split_once(']')?;
            (host, suffix.strip_prefix(':'))
        }
        None => match authority.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        },
    };
    if host.is_empty() {
        return None;
    }
    let port = match port {
        Some(port) => port.parse().ok()?,
        None => match scheme.as_str() {
            "https" => 443,
            "http" => 80,
            _ => return None,
        },
    };
    Some((scheme, host.to_ascii_lowercase(), port))
}

/// A Git status path as text for a status message: valid UTF-8 kept as is,
/// every byte that is not written as `\xNN`, so a name like `caf\xe9.md`
/// stays recognisable instead of collapsing to a replacement character.
pub(crate) fn escaped_path(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len());
    for chunk in bytes.utf8_chunks() {
        text.push_str(chunk.valid());
        for byte in chunk.invalid() {
            text.push_str(&format!("\\x{byte:02x}"));
        }
    }
    text
}

/// Whether a repository-relative status path, as bytes, lies inside the Vault
/// at `vault_relative`. Compared by path component, so `notes-old/a.md` is not
/// inside `notes`. Works on bytes because a path that is not valid UTF-8 still
/// has to be placed inside or outside the Vault (#638).
pub(crate) fn status_bytes_in_vault(path: &[u8], vault_relative: &std::path::Path) -> bool {
    let Some(prefix) = vault_relative.to_str() else {
        return false;
    };
    if prefix.is_empty() {
        return true;
    }
    path.strip_prefix(prefix.as_bytes())
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(b"/"))
}

/// A remote that accepts connections and then never sends a byte: the
/// pathology [`bound_network_waits`] exists for. Returns an HTTPS URL for it.
#[cfg(test)]
pub(crate) fn stalled_https_remote() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind stalled remote");
    let port = listener
        .local_addr()
        .expect("stalled remote address")
        .port();
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for connection in listener.incoming().flatten() {
            held.push(connection);
        }
    });
    format!("https://127.0.0.1:{port}/vault.git")
}

/// Whether this source makes local commits of its own, and so has a
/// `VaultWorkKind::Commit` turn (issue #267).
///
/// True for Local history, which is nothing but local commits, and for
/// Two-way, whose commit is the half of its sync that needs no remote. False
/// for Pull-only, which refuses writes and must leave a folder its operator
/// dirtied alone, and for a plain local folder, which has no Git at all.
///
/// Deliberately separate from `VaultSource::managed_git_poll_interval`, which
/// answers a different question, does this Vault poll a remote, and stays
/// exactly as it is. Both live here rather than on `VaultSource` itself
/// because the answer is a fact about Git behaviour, not about the registry
/// record, and this boundary is what owns Git behaviour.
pub fn source_commits(source: &VaultSource) -> bool {
    match source {
        VaultSource::Local { .. } => false,
        VaultSource::ExistingGit { mode, .. } | VaultSource::ManagedGit { mode, .. } => {
            matches!(mode, VaultGitMode::LocalHistory | VaultGitMode::TwoWay)
        }
    }
}

/// Whether this source has a remote to synchronize with, which is what
/// separates a Vault whose console offers **Sync now** from one that can only
/// be offered **Commit now**.
pub fn source_syncs_remote(source: &VaultSource) -> bool {
    match source {
        VaultSource::Local { .. } => false,
        VaultSource::ExistingGit { mode, .. } | VaultSource::ManagedGit { mode, .. } => {
            matches!(mode, VaultGitMode::PullOnly | VaultGitMode::TwoWay)
        }
    }
}

#[cfg(test)]
mod network_timeout_tests {
    #[test]
    fn network_waits_are_bounded_once_configured() {
        super::bound_network_waits();
        // SAFETY: reads of the process-global integers set above.
        let (connect, transfer) = unsafe {
            (
                git2::opts::get_server_connect_timeout_in_milliseconds().expect("connect"),
                git2::opts::get_server_timeout_in_milliseconds().expect("transfer"),
            )
        };
        assert_eq!(connect, 2_000);
        assert_eq!(transfer, 2_000);
    }
}

#[cfg(test)]
mod credential_tests {
    use super::{callback_origin_matches, escaped_path, status_bytes_in_vault, url_origin};
    use std::path::Path;

    #[test]
    fn origins_compare_scheme_host_and_port_with_defaults_filled_in() {
        assert_eq!(
            url_origin("https://Git.Example.com/team/notes.git"),
            url_origin("https://git.example.com:443/other")
        );
        assert_ne!(
            url_origin("https://git.example.com/notes.git"),
            url_origin("http://git.example.com/notes.git")
        );
        assert_ne!(
            url_origin("https://git.example.com/notes.git"),
            url_origin("https://git.example.com:8443/notes.git")
        );
        assert_ne!(
            url_origin("https://git.example.com/notes.git"),
            url_origin("https://elsewhere.example.com/notes.git")
        );
        assert_eq!(
            url_origin("https://[::1]:8443/notes.git"),
            Some(("https".into(), "::1".into(), 8443))
        );
        assert_eq!(url_origin("not a url"), None);
    }

    #[test]
    fn an_ipv6_remote_matches_the_bracketless_url_libgit2_hands_the_callback() {
        let default_port = url_origin("https://[fd00::5]/notes.git").expect("origin");
        assert!(callback_origin_matches(
            "https://fd00::5/notes.git",
            &default_port
        ));
        assert!(!callback_origin_matches(
            "https://fd00::6/notes.git",
            &default_port
        ));
        let other_port = url_origin("https://[fd00::5]:8443/notes.git").expect("origin");
        assert!(callback_origin_matches(
            "https://fd00::5:8443/notes.git",
            &other_port
        ));
        assert!(!callback_origin_matches(
            "https://fd00::5/notes.git",
            &other_port
        ));
    }

    #[test]
    fn undecodable_path_bytes_are_escaped_and_the_rest_kept() {
        assert_eq!(escaped_path(b"caf\xe9-latin1.md"), "caf\\xe9-latin1.md");
        assert_eq!(
            escaped_path("été/\u{1F600}.md".as_bytes()),
            "été/\u{1F600}.md"
        );
    }

    #[test]
    fn status_bytes_are_placed_in_the_vault_by_component() {
        assert!(status_bytes_in_vault(
            b"notes/caf\xe9.md",
            Path::new("notes")
        ));
        assert!(!status_bytes_in_vault(
            b"notes-old/a.md",
            Path::new("notes")
        ));
        assert!(status_bytes_in_vault(b"anything\xff", Path::new("")));
    }
}

/// A plain-HTTP server for tests that talks to libgit2's real HTTP transport,
/// so the credential tests exercise libgit2's own replay and redirect
/// behaviour rather than a synthetic `ErrorCode::Auth` (#638).
#[cfg(test)]
pub(crate) mod test_http {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    /// One request as the server read it.
    #[derive(Clone, Debug)]
    pub(crate) struct Request {
        pub method: String,
        pub target: String,
        pub authorization: Option<String>,
        pub content_type: Option<String>,
        pub body: Vec<u8>,
    }

    pub(crate) struct Server {
        pub base: String,
        requests: Arc<Mutex<Vec<Request>>>,
    }

    impl Server {
        /// Serve every connection with `handler`, one request per connection.
        pub(crate) fn start(handler: impl Fn(&Request) -> Vec<u8> + Send + 'static) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind test HTTP server");
            let base = format!(
                "http://127.0.0.1:{}",
                listener.local_addr().expect("address").port()
            );
            let requests = Arc::new(Mutex::new(Vec::new()));
            let seen = Arc::clone(&requests);
            std::thread::spawn(move || {
                for stream in listener.incoming().flatten() {
                    let Ok(mut writer) = stream.try_clone() else {
                        continue;
                    };
                    let Some(request) = read_request(&mut BufReader::new(stream)) else {
                        continue;
                    };
                    seen.lock().expect("requests").push(request.clone());
                    let _ = writer.write_all(&handler(&request));
                    let _ = writer.flush();
                }
            });
            Self { base, requests }
        }

        pub(crate) fn requests(&self) -> Vec<Request> {
            self.requests.lock().expect("requests").clone()
        }
    }

    fn read_request(reader: &mut impl BufRead) -> Option<Request> {
        let mut line = String::new();
        reader.read_line(&mut line).ok()?;
        let mut parts = line.split_whitespace();
        let method = parts.next()?.to_string();
        let target = parts.next()?.to_string();
        let (mut authorization, mut content_type) = (None, None);
        let (mut length, mut chunked) = (0usize, false);
        loop {
            let mut header = String::new();
            reader.read_line(&mut header).ok()?;
            let header = header.trim_end();
            if header.is_empty() {
                break;
            }
            let (name, value) = header.split_once(':')?;
            let value = value.trim().to_string();
            match name.to_ascii_lowercase().as_str() {
                "authorization" => authorization = Some(value),
                "content-type" => content_type = Some(value),
                "content-length" => length = value.parse().ok()?,
                "transfer-encoding" => chunked = value.eq_ignore_ascii_case("chunked"),
                _ => {}
            }
        }
        let mut body = Vec::new();
        if chunked {
            loop {
                let mut size = String::new();
                reader.read_line(&mut size).ok()?;
                let size = usize::from_str_radix(size.trim().split(';').next()?, 16).ok()?;
                let mut chunk = vec![0; size + 2];
                reader.read_exact(&mut chunk).ok()?;
                if size == 0 {
                    break;
                }
                body.extend_from_slice(&chunk[..size]);
            }
        } else {
            body.resize(length, 0);
            reader.read_exact(&mut body).ok()?;
        }
        Some(Request {
            method,
            target,
            authorization,
            content_type,
            body,
        })
    }

    /// A complete response that closes the connection.
    pub(crate) fn respond(status: &str, headers: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
        let mut response = format!("HTTP/1.1 {status}\r\nConnection: close\r\n");
        for (name, value) in headers {
            response.push_str(&format!("{name}: {value}\r\n"));
        }
        response.push_str(&format!("Content-Length: {}\r\n\r\n", body.len()));
        let mut response = response.into_bytes();
        response.extend_from_slice(body);
        response
    }

    /// 401 with a Basic challenge.
    pub(crate) fn unauthorized() -> Vec<u8> {
        respond(
            "401 Unauthorized",
            &[("WWW-Authenticate", "Basic realm=\"test\"")],
            b"",
        )
    }

    /// The `Authorization` header a client sends for `username:password`.
    pub(crate) fn basic(username: &str, password: &str) -> String {
        use base64::Engine;
        format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(format!("{username}:{password}"))
        )
    }

    /// A smart-HTTP Git server over the bare repositories under `root`,
    /// answering through `git http-backend` and refusing every request that
    /// does not carry `expected` as its `Authorization`.
    pub(crate) fn git_backend(
        root: PathBuf,
        expected: String,
    ) -> impl Fn(&Request) -> Vec<u8> + Send + 'static {
        move |request| {
            if request.authorization.as_deref() != Some(expected.as_str()) {
                return unauthorized();
            }
            let (path, query) = request
                .target
                .split_once('?')
                .unwrap_or((&request.target, ""));
            let mut child = std::process::Command::new("git")
                .arg("http-backend")
                .env("GIT_PROJECT_ROOT", &root)
                .env("GIT_HTTP_EXPORT_ALL", "1")
                .env("REMOTE_USER", "tester")
                .env("REQUEST_METHOD", &request.method)
                .env("PATH_INFO", path)
                .env("QUERY_STRING", query)
                .env(
                    "CONTENT_TYPE",
                    request.content_type.as_deref().unwrap_or(""),
                )
                .env("CONTENT_LENGTH", request.body.len().to_string())
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .spawn()
                .expect("git http-backend");
            child
                .stdin
                .take()
                .expect("stdin")
                .write_all(&request.body)
                .expect("request body");
            let output = child.wait_with_output().expect("http-backend output");
            let split = output
                .stdout
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .map(|at| (at, 4))
                .or_else(|| {
                    output
                        .stdout
                        .windows(2)
                        .position(|window| window == b"\n\n")
                        .map(|at| (at, 2))
                })
                .expect("CGI header block");
            let head = String::from_utf8_lossy(&output.stdout[..split.0]).into_owned();
            let body = &output.stdout[split.0 + split.1..];
            let mut status = "200 OK".to_string();
            let mut headers = Vec::new();
            for line in head.lines() {
                let (name, value) = line.split_once(':').expect("CGI header");
                if name.eq_ignore_ascii_case("status") {
                    status = value.trim().to_string();
                } else {
                    headers.push((name.to_string(), value.trim().to_string()));
                }
            }
            let headers = headers
                .iter()
                .map(|(name, value)| (name.as_str(), value.as_str()))
                .collect::<Vec<_>>();
            respond(&status, &headers, body)
        }
    }
}
