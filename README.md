# ipabox

A native macOS app for sideloading `.ipa` files onto **Apple TV (tvOS 17+)**, iPhone and iPad, signed with a free Apple ID. No paid Developer Program membership and no Xcode required.

ipabox pairs with an Apple TV over the network using the RemoteXPC protocol that tvOS 17 introduced, signs your IPA with your Apple ID and pushes it through a local tunnel. It also re-signs installed apps automatically before the 7-day free certificate expires.

The UI is written in Rust with [gpui](https://github.com/zed-industries/zed). Signing is done by `plumesign` and device work by `pymobiledevice3`.

## Requirements

- A Mac with Apple Silicon running macOS 12 or later
- An Apple ID (a free account works)
- Python 3 with [pipx](https://pipx.pypa.io)
- [`pymobiledevice3`](https://github.com/doronz88/pymobiledevice3), installed with pipx
- Your Mac admin password, once per boot, to start the install tunnel
- The Apple TV and the Mac on the same local network

## Install

### 1. Install pipx and pymobiledevice3

ipabox calls `pymobiledevice3` for pairing and installing. Install it with pipx:

```sh
brew install pipx
pipx ensurepath
pipx install pymobiledevice3
```

Check that it works:

```sh
pymobiledevice3 version
```

ipabox looks for the pipx environment in these locations, so the default pipx setup works out of the box:

- `~/Library/Application Support/pipx/venvs/pymobiledevice3` (current pipx on macOS)
- `~/.local/pipx/venvs/pymobiledevice3`
- `~/.local/share/pipx/venvs/pymobiledevice3`
- any `pymobiledevice3` found on `PATH` (including `~/.local/bin`)

If you see `pymobiledevice3 venv not found`, pymobiledevice3 is missing or was installed with plain `pip`. Reinstall it with `pipx install pymobiledevice3`.

### 2. Get the app

Download `ipabox.app` from the Releases page and drag it to `/Applications`, or build it yourself (see [Build from source](#build-from-source)).

The app is ad-hoc signed. If macOS refuses to open it the first time, right-click the app and choose **Open**.

On first launch macOS asks for **Local Network** access. Allow it, or ipabox cannot see your devices.

## Usage

### Settings

1. Enter your Apple ID **email** and **password**, then click **Save**. Use your normal account password, not an app-specific password.
2. Click **Sign in**. Apple sends a verification code to your other devices. Approve the "new sign-in" prompt and type the code into ipabox.
3. Click **Load teams** and pick your team. A free account has a single personal team.
4. Under **Install Tunnel**, click **Start (admin)** and enter your Mac password. This starts `pymobiledevice3 remote tunneld` as root, which tvOS 17+ and iOS 17+ need for installs. It keeps running until you restart the Mac.

### Pair an Apple TV

1. On the Apple TV, open **Settings > Remotes and Devices > Remote App and Devices** and keep that screen open.
2. In ipabox, open **Devices**. The Apple TV appears with a **Pair** button.
3. Click **Pair**. The Apple TV shows a 6-digit PIN. Type it into ipabox.
4. The button changes to **Paired**. You only do this once per device.

iPhone and iPad: connect the device with a cable and tap **Trust** on it.

### Install an IPA

1. Open **Install** and pick the target device at the top.
2. Click the drop zone and choose an `.ipa`.
3. ipabox registers the device with your team, signs the IPA and installs it over the tunnel. The log shows each step.

Installed apps are listed under **Installed**, where you can **Re-sign** or **Reinstall** them.

### Automatic re-signing

Apps signed with a free Apple ID stop opening after 7 days. While ipabox is running, it checks every hour and re-signs and reinstalls any app that is close to expiring. This needs the tunnel to be running and the device to be reachable.

## Build from source

You need the Rust toolchain and the `plumesign` binary.

```sh
brew install rustup pipx
rustup-init
pipx install pymobiledevice3
```

### plumesign

`plumesign` is not in this repository because of its size (about 84 MB). Place an arm64 build at `vendor/plumesign`. ipabox uses these subcommands: `account login`, `account list`, `account teams`, `account register-device` and `sign --apple-id`.

It comes from [PlumeImpactor](https://github.com/claration/Impactor) and [omnisette](https://github.com/SideStore/omnisette). To log in with current Apple servers it needs the same changes as Impactor 2.7.2:

- `X-Mme-Client-Info` must identify as `com.apple.AuthKit/1 (com.apple.akd/1.0)`, not Xcode. With the old Xcode value Apple answers `SRP error 503`.
- The anisette v3 server must be `https://mac-ani.jkcoxson.com`. With older public servers Apple answers `SRP error -80045`.

### Build the app

```sh
./package.sh
```

This builds a release binary, assembles `dist/ipabox.app`, copies `vendor/plumesign` into it and ad-hoc signs the bundle. Install it with:

```sh
ditto dist/ipabox.app /Applications/ipabox.app
```

`package.sh` regenerates the app icon with `tools/make_icon.py`, which needs Pillow:

```sh
python3 -m pip install --user pillow
```

For development you can run it directly with `cargo run`. Put `plumesign` next to the binary, in `$TMPDIR/ipabox/`, or on `PATH`.

## Where data lives

| What | Where |
| --- | --- |
| Settings, app database, IPA cache, logs | `~/Library/Application Support/ipabox/` |
| Apple ID session and anisette identity | `~/.config/PlumeImpactor/` |
| RemotePairing pair records | `~/.pymobiledevice3/` |

To reset the Apple ID session, quit ipabox and move `~/.config/PlumeImpactor/state.plist` out of the way. A new identity is created on the next sign-in.

## Troubleshooting

**`Authentication SRP error 503` or `-80045` when signing in**
Your `plumesign` is missing the login fixes described in [plumesign](#plumesign). Repeated failed attempts can also make Apple throttle your account for a few hours, so stop retrying for a while.

**`pymobiledevice3 venv not found`**
Install it with pipx (see [Install](#1-install-pipx-and-pymobiledevice3)).

**Pairing fails with `ConnectionTerminatedError`**
The Apple TV left pairing mode. Keep **Remote App and Devices** open on the TV for the whole pairing, then try again.

**Pairing fails with `Permission denied: ~/.pymobiledevice3/...`**
An older version created that folder as root. Quit ipabox, delete the folder if it is empty (or fix its owner with `sudo chown -R "$USER" ~/.pymobiledevice3`) and pair again.

**`Tunneld not running`**
Click **Start (admin)** in Settings. You need to do this again after every restart.

**`Developer API error 4550 ... Program License Agreement`**
Sign in at <https://developer.apple.com/account> and accept the pending agreement, or leave any team where you are only a member.

**The device does not show up**
Check that the Mac and the Apple TV are on the same network, that Local Network access is allowed for ipabox in **System Settings > Privacy & Security > Local Network**, and click refresh.

## Credits

ipabox builds on the work of:

- [bitxeno/atvloadly](https://github.com/bitxeno/atvloadly)
- [claration/Impactor](https://github.com/claration/Impactor) (plumesign)
- [SideStore/omnisette](https://github.com/SideStore/omnisette)
- [doronz88/pymobiledevice3](https://github.com/doronz88/pymobiledevice3)
- [zed-industries/zed](https://github.com/zed-industries/zed) (gpui)

## License

MIT. See [LICENSE](LICENSE).
