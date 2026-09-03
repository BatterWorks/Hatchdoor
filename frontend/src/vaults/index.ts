/**
 * The Vault collection client: the one module every surface asks about the
 * Vault collection. It owns the Vault list, the per-Vault note counts, the
 * demo-mode projection of each Vault's slot, and the SSE revision stream that
 * invalidates all three. Per-surface presentation stays in the surfaces.
 *
 * Nothing outside this directory should fetch `GET /api/v1/vaults`,
 * `GET /api/v1/vaults/all/stats`, or subscribe to `/api/v1/vaults/events`.
 */
export { useVaultCollection, useVaultProjection } from "./useVaultCollection";
export {
  fetchRegistryRevision,
  refreshVaultCollection,
  type VaultCollectionState,
} from "./vaultCollectionStore";
export { type VaultProjection } from "./vaultProjection";
