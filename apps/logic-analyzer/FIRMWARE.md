# FX2 firmware (fx2lafw): licence and how the app gets it

Cypress EZ-USB FX2(LP) logic analyzers keep no firmware of their own. The host loads a firmware
image into the chip's RAM every time the board is plugged in. Voltrus Logic Analyzer uses
**fx2lafw**, the open firmware of the sigrok project. This file records what its licence allows
and what the app does as a result. Checked on 2026-11-10.

## The licence

- fx2lafw is "licensed under the terms of the GNU General Public License (GPL), version 2 or
  later". It includes helper code (fx2lib, plus the Hantek 6022BE and SainSmart DDS120 firmware)
  under the LGPL 2.1 or later, but "the firmware as a whole is licensed under the terms of the
  GPLv2+". Source: the project README,
  <https://github.com/sigrokproject/sigrok-firmware-fx2lafw/blob/master/README>, and the wiki,
  <https://sigrok.org/wiki/Fx2lafw>.
- The binary release (`sigrok-firmware-fx2lafw-bin-0.1.7.tar.gz`, 2019-11-15) ships the `.fw`
  images together with `COPYING` (GPL-2.0) and `COPYING.LESSER` (LGPL-2.1):
  <https://sigrok.org/download/binary/sigrok-firmware-fx2lafw/>. The matching source tarball is
  <https://sigrok.org/download/source/sigrok-firmware-fx2lafw/sigrok-firmware-fx2lafw-0.1.7.tar.gz>.
- Debian, Fedora and FreeBSD all package it as GPL-2.0-or-later and install the `.fw` files as
  separate data files (`/usr/share/sigrok-firmware`):
  <https://salsa.debian.org/electronics-team/sigrok/sigrok-firmware-fx2lafw/-/blob/master/COPYING>,
  <https://packages.fedoraproject.org/pkgs/sigrok-firmware-fx2lafw/sigrok-firmware-fx2lafw/>,
  <https://www.freshports.org/science/sigrok-firmware-fx2lafw/>.

## Does it touch the app's licence?

No, as long as the app does not contain fx2lafw code. The firmware runs on a separate processor,
the FX2's 8051, and the app talks to it over USB using vendor requests (`0xB0` version, `0xB1`
start, `0xB2` chip revision) and a bulk endpoint. These are two separate programs that
communicate at arm's length. The GPL FAQ treats that as mere aggregation, not a combined work:
<https://www.gnu.org/licenses/gpl-faq.html#MereAggregation>. So the app's own licence (the
GPL-3.0 source, and the separate commercial builds JoyoDigitama can sell because of the CLA) is
not affected.

The app does not link or copy libsigrok, PulseView or fx2lafw code. The driver in
`crates/mt-logic/src/fx2` was written from the interface facts alone: the request codes and the
3-byte start command (flags, delay high, delay low) in fx2lafw's `include/command.h`, endpoint 2
bulk IN, the 30/48 MHz clock and divider rules, and the FX2's own documented firmware-load
request `0xA0` at `CPUCS` `0xE600`.

## Could the app ship the firmware file inside its bundle?

Legally yes, with obligations, and in practice not cleanly:

1. **Shipping it is distribution under GPLv2.** Every copy would have to come with the complete
   corresponding source, or a written offer valid for three years (GPLv2 section 3). A link to
   sigrok's server only satisfies section 3(c), which is for non-commercial distribution. We
   would have to host the exact source ourselves, keep it available, and include the licence
   text in each bundle. At least one vendor does exactly this: the "Analisador" automotive
   analyzer embeds fx2lafw 0.1.7 and publishes the unmodified source to meet the GPL
   (<https://github.com/express-bus-developer/fx2lafw-fonte>). So aggregation with a source
   mirror is a model people use.
2. **The Mac App Store is the real problem.** Apple's usage rules put restrictions on the
   recipient that GPL section 6 forbids ("You may not impose any further restrictions"). The FSF
   made this argument publicly when it had GNU Go removed from the App Store
   (<https://www.fsf.org/news/2010-05-app-store-compliance>,
   <https://www.fsf.org/blogs/licensing/more-about-the-app-store-gpl-enforcement>). For a GPL work
   we do not own, that conflict cannot be resolved; the CLA only covers code contributed to this
   repository.

## Decision: the app does not ship the firmware

The app gets fx2lafw in one of three ways, all chosen by the user:

1. **Already installed.** It looks in the folders sigrok packages and PulseView install to:
   `/usr/share/sigrok-firmware`, `/usr/local/share/sigrok-firmware`,
   `/opt/homebrew/share/sigrok-firmware`, `~/.local/share/sigrok-firmware`, PulseView.app on
   macOS, and the PulseView install folder on Windows.
2. **Download from sigrok.org.** A button in Device → Firmware fetches the official 0.1.7 binary
   tarball straight from sigrok.org. It is checked against its SHA-256 (`c876fd07…1723af`) and so
   is each image. The images are unpacked into the app's data folder along with `COPYING`,
   `COPYING.LESSER`, `README` and a `SOURCE.txt` that names the source tarball. The user's
   computer downloads it from sigrok; we never distribute it. The download happens only on that
   click.
3. **Choose a file.** Any `.fw` image, for example a newer release or the user's own build. An
   image that is not byte-identical to the 0.1.7 release is used, and the app labels it "not the
   official image".

This keeps every distribution channel (Mac App Store, Polar direct download, source) free of
third-party GPL files, and the app needs no source mirror. The cost is one extra step on the
first connect. For a Mac App Store build, the sandbox allows the download into the app container,
since `network.client` is already granted.

If the owner would rather bundle the firmware in the direct-download DMG only, it is doable.
Include the `.fw` files plus `COPYING`/`COPYING.LESSER`, host
`sigrok-firmware-fx2lafw-0.1.7.tar.gz` on voltrus.id for at least three years, and add a written
offer to the About box. Keep it out of the App Store build. That is a business decision, not a
technical one.

## Pinned hashes (release 0.1.7)

`crates/mt-logic/src/fx2/firmware.rs` (`KNOWN`, `TARBALL_SHA256`) holds the SHA-256 of the
tarball and of each 8-bit logic analyzer image. They were computed from the file downloaded from
sigrok.org on 2026-11-10.
