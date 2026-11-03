# Signed macOS releases

RSQL can keep distributing DMGs through GitHub Releases. Sign the app with a **Developer ID Application** certificate, submit it to Apple's notarization service, and attach the resulting ticket to the app. This is the route for normal Gatekeeper approval outside the Mac App Store. Users may still see the standard first-launch confirmation for an app downloaded from the internet, but should not need `xattr` or a security override. [Apple Developer ID](https://developer.apple.com/developer-id/)

## Project status

- The release workflow already builds Apple Silicon and Intel macOS installers separately.
- `src-tauri/tauri.conf.json` already enables the hardened runtime and uses the stable bundle identifier `com.rust-dd.rsql`.
- Updater signatures are configured, but they authenticate updates to RSQL itself; they do not establish trust with Gatekeeper.
- The Developer ID Application certificate and private key for Gympiper are configured in the repository's signing secrets. Notarization credentials must also be present before enabling signing. A signed release still needs to pass the checks below before publication.
- `.github/workflows/release.yml` now supports signing and notarization when the repository variable `MACOS_SIGNING_ENABLED` is `true`. Until then, macOS builds remain unsigned and emit a workflow warning. Enabled signing fails on missing credentials instead of falling back to an unsigned release.

## One-time Apple setup

An existing Apple Developer Program membership is sufficient. For a new membership, Apple lists 99 USD per year, with local pricing during enrollment. Free development signing does not provide this distribution route. [Membership](https://developer.apple.com/programs/enroll/)

1. Create a certificate signing request in Keychain Access on your Mac.
2. Have the team's Account Holder create a **Developer ID Application** certificate in [Certificates, Identifiers & Profiles](https://developer.apple.com/account/resources/certificates/list). An Apple Development, Apple Distribution, or Developer ID Installer certificate is not the application certificate needed for these DMGs.
3. Download and install the certificate on the Mac that created the request, so it is paired with its private key.
4. In Keychain Access → My Certificates, export the certificate and its private key together as a password-protected `.p12` file. Keep it outside the repository.
5. Get the exact signing identity and prepare a single-line certificate value:

   ```bash
   security find-identity -v -p codesigning
   openssl base64 -A -in /path/to/developer-id.p12 -out /path/to/certificate-base64.txt
   ```

6. Create an [app-specific password](https://support.apple.com/102654) for the Apple account used for notarization. Use this password, not the account's normal login password. Find the Team ID on the [membership page](https://developer.apple.com/account/#/membership).

Tauri supports these credentials directly and handles certificate import, signing, notarization and stapling during bundling. It also supports App Store Connect API keys; this workflow uses Apple ID authentication, so no `.p8` key file is needed. [Tauri macOS signing](https://v2.tauri.app/distribute/sign/macos/)

Tauri notarizes the `.app`, but its DMG bundler only signs the outer disk image. The workflow therefore submits the finished DMG separately, requires an `Accepted` result, staples its ticket and checks Gatekeeper before replacing the draft's DMG asset. Apple recommends notarizing the outermost distribution container. [Tauri DMG implementation](https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.12.1/crates/tauri-bundler/src/bundle/macos/dmg/mod.rs), [Apple distribution guidance](https://developer.apple.com/forums/thread/125145)

## GitHub configuration

In [repository Actions settings](https://github.com/rust-dd/rsql/settings/secrets/actions), add these repository secrets:

| Secret | Value |
| --- | --- |
| `APPLE_CERTIFICATE` | Contents of the single-line `certificate-base64.txt` file |
| `APPLE_CERTIFICATE_PASSWORD` | Password used when exporting the `.p12` |
| `APPLE_SIGNING_IDENTITY` | Exact identity, such as `Developer ID Application: Your Name (TEAMID)` |
| `APPLE_ID` | Apple account email used for notarization |
| `APPLE_PASSWORD` | App-specific password for that account |
| `APPLE_TEAM_ID` | Developer team's ID |

Then add the **repository variable** `MACOS_SIGNING_ENABLED` with the value `true`. The signing identity and Team ID must refer to the same team. Existing Tauri updater secrets are still required. Keep certificate contents and passwords out of source files, release descriptions and chat.

## First signed release

1. Merge the icon and workflow changes, then update the version consistently as for any other release.
2. Push the new `v*` tag. A manual workflow dispatch must target a tag containing the updated workflow; rebuilding an old tag does not automatically use these changes.
3. Wait for both macOS jobs to finish. The workflow checks the app signature, Gatekeeper assessment and stapled ticket, then notarizes and verifies the DMG and replaces its draft asset. The initial upload precedes that final replacement, so do not publish the **draft** while jobs are running or if any job fails.
4. Download the DMG with a browser on a clean Mac or VM. Copy RSQL to Applications and open it normally, without clearing quarantine. Check the icon, a database connection, the SQL editor, the terminal and the updater. Cover Apple Silicon and Intel separately.
5. Publish the draft after verification. Record that this is the first signed and notarized release and update the README installation instructions. Older unsigned downloads do not become trusted retroactively.

For a downloaded and installed app, these checks should succeed:

```bash
codesign --verify --deep --strict --verbose=2 /Applications/rsql.app
spctl --assess --type execute --verbose=4 /Applications/rsql.app
xcrun stapler validate /Applications/rsql.app
```

Also check the downloaded installer itself (substitute its actual path):

```bash
xcrun stapler validate /path/to/rsql.dmg
spctl --assess --type open --context context:primary-signature --verbose=4 /path/to/rsql.dmg
```

`spctl` should report acceptance with a notarized Developer ID source. If notarization fails, inspect Apple's submission log before retrying. Do not disable the hardened runtime or clear quarantine to make a release check pass. [Apple notarization](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution)

The Mac App Store is a separate distribution option with its own requirements. It is not needed to solve this project's current installation warning.
