# macOS code signing + notarization (and optional Windows signing)

A macOS app downloaded from the internet is blocked by Gatekeeper unless it is
**signed with a Developer ID Application certificate and notarized** by Apple.
This is the only part of releasing that needs paid Apple credentials. Without it,
Mother Claude still builds — just **unsigned** (users right-click → **Open** once).

`tauri-action` does the signing, notarization, and stapling for us when the six
secrets below are present (see `.github/workflows/release.yml`). Mother Claude
needs **no custom notarization script** — unlike apps that bundle frozen
frameworks, its only extra payload is the Node sidecar, which the hardened-runtime
[entitlements](../src-tauri/entitlements.plist) already cover.

## What you need (one-time, requires the paid Apple Developer Program)

1. **A Developer ID Application certificate.**
   - Xcode → Settings → Accounts → Manage Certificates → **+** → *Developer ID
     Application* (or create it at <https://developer.apple.com/account/resources/certificates>).
   - In **Keychain Access**, find it, right-click → **Export** as a `.p12`, set a
     password. Then base64-encode it:
     ```bash
     base64 -i DeveloperID_Application.p12 | pbcopy   # now in your clipboard
     ```
2. **The signing identity string** — exactly as Keychain shows it, e.g.
   `Developer ID Application: Your Name (AB12CD34EF)`.
3. **Your Team ID** — the 10-character code in parentheses above (also at
   developer.apple.com → Membership).
4. **An app-specific password** for notarization — <https://appleid.apple.com> →
   Sign-In and Security → App-Specific Passwords. (Not your Apple ID password.)

## The six GitHub secrets

| Secret | Value |
|---|---|
| `APPLE_CERTIFICATE` | base64 of the `.p12` (step 1) |
| `APPLE_CERTIFICATE_PASSWORD` | the `.p12` export password |
| `APPLE_SIGNING_IDENTITY` | `Developer ID Application: Your Name (TEAMID)` |
| `APPLE_ID` | your Apple ID email |
| `APPLE_PASSWORD` | the **app-specific** password (step 4) |
| `APPLE_TEAM_ID` | your 10-char Team ID |

Add them under **Settings → Secrets and variables → Actions → Secrets**. When
`APPLE_CERTIFICATE` is empty, the workflow skips signing and macOS ships unsigned
(the release notes already tell users to right-click → Open).

## Local signed build

To produce a signed + notarized `.dmg` from your own Mac (no CI), export the same
values as environment variables, then build — Tauri picks them up automatically:

```bash
export APPLE_CERTIFICATE="$(base64 -i DeveloperID_Application.p12)"
export APPLE_CERTIFICATE_PASSWORD="…"
export APPLE_SIGNING_IDENTITY="Developer ID Application: Your Name (AB12CD34EF)"
export APPLE_ID="you@example.com"
export APPLE_PASSWORD="abcd-efgh-ijkl-mnop"   # app-specific
export APPLE_TEAM_ID="AB12CD34EF"
npm run tauri:build
```

Verify the result:

```bash
codesign --verify --deep --strict --verbose=2 "src-tauri/target/release/bundle/macos/Mother Claude.app"
spctl -a -vvv -t install "src-tauri/target/release/bundle/macos/Mother Claude.app"
xcrun stapler validate src-tauri/target/release/bundle/dmg/*.dmg
```

### Notarize + staple the .dmg (not just the .app)

`tauri build` notarizes and staples the **.app**, but it does **not** notarize the
**.dmg** wrapper. An unstapled dmg is `rejected` by Gatekeeper on download
(`spctl -a -t open …` → `source=Unnotarized Developer ID`), so the dmg must get
its own pass:

```bash
xcrun notarytool submit "src-tauri/target/release/bundle/dmg/Mother Claude_0.1.0_aarch64.dmg" \
  --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID" --wait
xcrun stapler staple "src-tauri/target/release/bundle/dmg/Mother Claude_0.1.0_aarch64.dmg"
```

`scripts/release/macos-release.sh` does this automatically after the build.

## Windows Authenticode (optional)

Windows installers ship **unsigned** today (SmartScreen → More info → Run anyway).
To sign them, get a code-signing certificate (OV/EV from a CA) and either:

- import the `.pfx` into the machine's certificate store and set
  `bundle.windows.certificateThumbprint` (+ `digestAlgorithm`, `timestampUrl`) in
  `src-tauri/tauri.conf.json`, **or**
- configure a custom `bundle.windows.signCommand` that invokes your signing tool.

Then add the import/sign step to the Windows leg of `.github/workflows/release.yml`.
This is a deliberate later addition; it is not required to publish.
