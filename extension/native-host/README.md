# Native messaging host — install paths and dev-key generation

This directory holds the host manifest template that the installer drops
onto disk and registers in the browser registries. The **extension-side
identity** — the RSA-2048 public key baked into `extension/manifest.json#key`
and the resulting stable extension ID — is what the host manifest's
`allowed_origins` references.

## Stable dev extension ID

```
blbgjagjodpiiclpecohlfhebgddkejn
```

This is derived from the public key DER bytes via the standard Chrome
algorithm (SHA-256 → first 16 bytes → hex → digit-to-letter remap
`0..f → a..p`). It is the single source of truth in
`crates/core/src/wire.rs::ALLOWED_DEV_EXTENSION_ID`. The corresponding
public key is baked into `extension/manifest.json#key`; the private half
lives at `extension/.dev-key.pem` and is intentionally **gitignored**.

Production builds rebuild the manifest with a different key (the Chrome
Web Store extension ID) — see *Release-time stamping* below.

## Release-time stamping

The committed host-manifest template at
`src-tauri/native-host/com.unduhin.host.json` ships with the dev
extension ID (`blbgjagjodpiiclpecohlfhebgddkejn`) baked into
`allowed_origins`. That is what every `cargo tauri build` packages by
default, and what the load-unpacked extension uses in development.

When releasing to the Chrome Web Store, `scripts/release.ps1` accepts a
`-WebStoreExtensionId <id>` parameter. If supplied, the script rewrites
the manifest's `chrome-extension://<id>/` entry in place before invoking
`cargo tauri build`, then restores the original on the way out (in a
`finally` block, so even a failed build leaves a clean working tree).
Until the first Web Store submission, leave the parameter unset and the
dev ID will ship.

```pwsh
# Dry-run a release with the production extension ID baked in.
scripts\release.ps1 -Version 0.4.0 `
    -WebStoreExtensionId abcdefghijklmnopabcdefghijklmnop
```

The ID is validated as 32 chars from `a-p` (Chrome's deterministic
extension-ID alphabet) before the substitution.

### Regenerating the dev key

If `.dev-key.pem` is missing or rotated, run from the repo root:

```pwsh
cd extension
openssl genrsa -out .dev-key.pem 2048

# Compute the new extension ID + public key for manifest.json:
node -e "
  const fs=require('fs'); const crypto=require('crypto');
  const der=fs.readFileSync('.dev-key.pem');
  const pub=crypto.createPublicKey(der).export({type:'spki',format:'der'});
  const id=crypto.createHash('sha256').update(pub).digest('hex').slice(0,32);
  let mapped=''; for (const c of id) mapped+=String.fromCharCode('a'.charCodeAt(0)+parseInt(c,16));
  console.log('EXT_ID =', mapped);
  console.log('KEY    =', pub.toString('base64'));
"
```

Then:

1. Paste the `KEY` value into `extension/manifest.json#key`.
2. Update `ALLOWED_DEV_EXTENSION_ID` in `crates/core/src/wire.rs` to the
   new `EXT_ID`.
3. Re-run the ts-rs export so the generated `.d.ts` files match:

   ```pwsh
   cargo test -p unduhin-core --features ts-rs-export export_wire_types
   ```

4. Commit the changes. The next `cargo tauri build` will register the
   correct ID in `allowed_origins` once the host-manifest template
   is in place.

## Per-OS host manifest paths

The extension itself does not install anything. The locations below are
documented here so the NSIS hook and any future Linux/macOS support land
in the right places.

### Windows (the one we actually ship)

Manifest file is staged under `$INSTDIR\native-host\com.unduhin.host.json`
by the installer. Each supported browser looks it up via a `HKCU` registry
key whose `(Default)` value is the absolute manifest path:

```
HKCU\Software\Google\Chrome\NativeMessagingHosts\com.unduhin.host
HKCU\Software\Microsoft\Edge\NativeMessagingHosts\com.unduhin.host
HKCU\Software\BraveSoftware\Brave-Browser\NativeMessagingHosts\com.unduhin.host
```

### macOS (documented for future port)

```
~/Library/Application Support/Google/Chrome/NativeMessagingHosts/com.unduhin.host.json
~/Library/Application Support/Microsoft Edge/NativeMessagingHosts/com.unduhin.host.json
~/Library/Application Support/BraveSoftware/Brave-Browser/NativeMessagingHosts/com.unduhin.host.json
```

### Linux (documented for future port)

```
~/.config/google-chrome/NativeMessagingHosts/com.unduhin.host.json
~/.config/microsoft-edge/NativeMessagingHosts/com.unduhin.host.json
~/.config/BraveSoftware/Brave-Browser/NativeMessagingHosts/com.unduhin.host.json
```

## Smoke-testing the host manually

Once the manifest is in place, you can drive the host from a
shell without a browser at all — handy for debugging framing without
the extension in the loop.

```pwsh
# PowerShell — write a single framed `ping` to the host and read its
# `pong` reply. Replace the path if your install is elsewhere.
$bin = "$env:LOCALAPPDATA\Programs\Unduhin\native-host\unduhin-native-host.exe"
$payload = '{"type":"ping"}'
$bytes = [System.Text.Encoding]::UTF8.GetBytes($payload)
$prefix = [System.BitConverter]::GetBytes([UInt32]$bytes.Length)

$proc = Start-Process $bin -NoNewWindow -RedirectStandardInput in.tmp `
  -RedirectStandardOutput out.tmp -PassThru
$stdin = [System.IO.File]::OpenWrite("in.tmp")
$stdin.Write($prefix, 0, 4); $stdin.Write($bytes, 0, $bytes.Length); $stdin.Close()

Start-Sleep -Milliseconds 500
$proc | Stop-Process
[System.IO.File]::ReadAllBytes("out.tmp") | Format-Hex | Select-Object -First 5
```

A Node version of the same smoke test:

```js
import { spawn } from "node:child_process";
const child = spawn(process.env.UNDUHIN_HOST ?? "unduhin-native-host.exe");
const payload = Buffer.from(JSON.stringify({ type: "ping" }), "utf8");
const prefix = Buffer.alloc(4);
prefix.writeUInt32LE(payload.length, 0);
child.stdin.write(Buffer.concat([prefix, payload]));
child.stdout.once("data", (b) => {
  const len = b.readUInt32LE(0);
  console.log("got:", b.slice(4, 4 + len).toString("utf8"));
  child.stdin.end();
});
```
