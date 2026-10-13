# macOS code signing + notarization (and optional Windows signing)

A macOS app downloaded from the internet is blocked by Gatekeeper unless it is
**signed with a Developer ID Application certificate and notarized** by Apple.
This is the only part of releasing that needs paid Apple credentials. Without it,
Mother Claude still builds — just **unsigned** (users right-click → **Open** once).

Releases are signed **on the Mac that builds them**
([RELEASING.md](RELEASING.md)): `scripts/release/release-macos.sh` signs with the
certificate in your login keychain, notarizes with the credentials in
`scripts/apple-credentials.env`, then notarizes and staples the `.dmg` too. The
bundle's only extra payload is the Node sidecar; its one native file, the Agent
SDK's `claude` binary, arrives already signed by Anthropic's Developer ID with
the hardened runtime and a secure timestamp, which notarization accepts. The
hardened-runtime [entitlements](../src-tauri/entitlements.plist) cover the rest.

## What you need (one-time, requires the paid Apple Developer Program)

1. **A Developer ID Application certificate** in your login keychain —
   Xcode → Settings → Accounts → Manage Certificates → **+** → *Developer ID
   Application* (or create it at <https://developer.apple.com/account/resources/certificates>).
   `security find-identity -v -p codesigning` lists it. (Only a CI build needs it
   exported: Keychain Access → right-click → **Export** as a `.p12`, then
   `base64 -i DeveloperID_Application.p12 | pbcopy`.)
2. **The signing identity string** — exactly as Keychain shows it, e.g.
   `Developer ID Application: Your Name (AB12CD34EF)`.
3. **Your Team ID** — the 10-character code in parentheses above (also at
   developer.apple.com → Membership).
4. **An app-specific password** for notarization — <https://appleid.apple.com> →
   Sign-In and Security → App-Specific Passwords. (Not your Apple ID password.)

## Local credentials

Copy `scripts/apple-credentials.env.example` to `scripts/apple-credentials.env`
(git-ignored) and fill in `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD`
(app-specific) and `APPLE_TEAM_ID`. `source` it before running the macOS leg.

## The six GitHub secrets (only if CI ever builds macOS)

| Secret | Value |
|---|---|
| `APPLE_CERTIFICATE` | base64 of the `.p12` (step 1) |
| `APPLE_CERTIFICATE_PASSWORD` | the `.p12` export password |
| `APPLE_SIGNING_IDENTITY` | `Developer ID Application: Your Name (TEAMID)` |
| `APPLE_ID` | your Apple ID email |
| `APPLE_PASSWORD` | the **app-specific** password (step 4) |
| `APPLE_TEAM_ID` | your 10-char Team ID |

Add them under **Settings → Secrets and variables → Actions → Secrets**. When
`APPLE_CERTIFICATE` is empty, the workflow skips signing and macOS ships unsigned.

## Local signed build

The release leg is the supported way (`source scripts/apple-credentials.env &&
RELEASE_TAG=v0.4.0 bash scripts/release/release-macos.sh`). By hand: with those
variables exported and the certificate in the keychain (no `APPLE_CERTIFICATE`
needed), `npx tauri build --target aarch64-apple-darwin` signs and notarizes the
`.app`. Verify the result:

```bash
B=src-tauri/target/aarch64-apple-darwin/release/bundle
codesign --verify --deep --strict --verbose=2 "$B/macos/Mother Claude.app"
spctl -a -vvv -t exec "$B/macos/Mother Claude.app"
xcrun stapler validate "$B"/dmg/*.dmg
```

### Notarize + staple the .dmg (not just the .app)

`tauri build` notarizes and staples the **.app**, but it does **not** notarize the
**.dmg** wrapper. An unstapled dmg is `rejected` by Gatekeeper on download
(`spctl -a -t open …` → `source=Unnotarized Developer ID`), so the dmg must get
its own pass:

```bash
DMG="src-tauri/target/aarch64-apple-darwin/release/bundle/dmg/Mother Claude_0.4.0_aarch64.dmg"
xcrun notarytool submit "$DMG" \
  --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID" --wait
xcrun stapler staple "$DMG"
```

`scripts/release/release-macos.sh` does this automatically after the build, and
fails unless `stapler validate` and `spctl` accept the result.

## Windows Authenticode (optional)

Windows installers ship **unsigned** today (SmartScreen → More info → Run anyway).
To sign them, get a code-signing certificate (OV/EV from a CA) and either:

- import the `.pfx` into the machine's certificate store and set
  `bundle.windows.certificateThumbprint` (+ `digestAlgorithm`, `timestampUrl`) in
  `src-tauri/tauri.conf.json`, **or**
- configure a custom `bundle.windows.signCommand` that invokes your signing tool.

`scripts/release/release-windows.ps1` runs `tauri build`, so a build that signs
produces signed installers with no change to the script — then check
`Get-AuthenticodeSignature` reports `Valid`, because a build that could not reach
the signing service can still produce an unsigned installer. Authenticode is
separate from the updater signature: a signed installer still needs its `.sig`.
This is a deliberate later addition; it is not required to publish.
