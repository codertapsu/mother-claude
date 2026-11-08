import { tr } from '../../core/i18n/i18n';
import { SecretStoreKind } from '../../core/models';
import { HostOs } from '../../core/paths';
import { secretStoreText } from '../../shared/accounts';

/**
 * Where the desktop keeps account tokens, in words: the store it names, else
 * its system's own (an older server does not say) — the macOS Keychain,
 * Windows Credential Manager, files only your user can read.
 */
export function tokenStoreText(kind: SecretStoreKind | undefined, os: HostOs): string {
  const store =
    kind ?? (os === 'macos' ? 'keychain' : os === 'windows' ? 'credentialManager' : 'files');
  return tr(secretStoreText(store));
}
