# Publishing HorsePower

Two things happen here: the source lives on GitHub, and the binaries are
delivered through a Launchpad PPA so `apt` handles updates.

```
GitHub  ──push──►  thepeakiscold/horsepower
                        │
                        │  python3 scripts/make-source.py --sign <KEYID>
                        ▼
                   dist/*.dsc + *.orig.tar.gz + *.debian.tar.xz
                        │
                        │  dput ppa:thepeakiscold/horsepower dist/*.dsc
                        ▼
            Launchpad builds amd64 on 26.04 (resolute)
                        │
                        ▼
   sudo add-apt-repository ppa:thepeakiscold/horsepower
   sudo apt install horsepower
```

## One-time setup

### 1. Launchpad account and PPA

1. Create an account at <https://launchpad.net/> (an Ubuntu One account works).
2. Sign the Ubuntu Code of Conduct — Launchpad refuses uploads without it.
   `https://launchpad.net/codeofconduct/2.0/+sign`
3. Create the PPA: `https://launchpad.net/~USERNAME/+activate-ppa`
   Name it `horsepower`.
4. Register your GPG key: `https://launchpad.net/~USERNAME/+editpgpkeys`
   Paste the output of:

   ```sh
   gpg --armor --export <FINGERPRINT>
   ```

   Launchpad emails an encrypted challenge; decrypt it with
   `gpg --decrypt` and paste the token back.

### 2. Signing key

This repository has already generated one:

| | |
| --- | --- |
| Fingerprint | `4CDB 0FC0 7720 09A8 A120  6A77 BC19 5B94 9E88 46BA` |
| Key ID | `4CDB0FC0772009A8A1206A77BC195B949E8846BA` |
| Passphrase | none (an upload key, not a personal identity key) |
| Public key | `packaging/horsepower-signing.asc` |
| Secret key backup | `~/horsepower-ppa-private-key.asc` (mode 0600) |

Keep the secret key off any machine you do not control. If it leaks, revoke it
with `gpg --gen-revoke`, upload the revocation to Launchpad and generate a new
key.

Import the pair on a new machine:

```sh
gpg --import packaging/horsepower-signing.asc
gpg --import ~/horsepower-ppa-private-key.asc
```

### 3. dput

```sh
sudo apt install dput
```

`~/.dput.cf`:

```ini
[horsepower]
fqdn                    = ppa.launchpad.net
method                  = ftp
incoming                = ~thepeakiscold/ubuntu/horsepower/
login                   = anonymous
allow_unsigned_uploads  = 0
```

## Release checklist

```sh
# 1. Bump the version in debian/changelog (top entry), src/horsepower/constants.py
#    and rust/horsepowerd/Cargo.toml, then:

export VERSION=1.0.1

# 2. Test everything
make test          # cargo test + python unittest
make lint          # clippy -D warnings, fmt --check, pyflakes

# 3. Build and sign the source package
export GNUPGHOME=$HOME/.gnupg
python3 scripts/make-source.py --sign 4CDB0FC0772009A8A1206A77BC195B949E8846BA

# 4. Upload (only the .dsc; Launchpad fetches the rest itself)
dput horsepower dist/horsepower_${VERSION}-0ubuntu1.dsc

# 5. Watch the build
xdg-open https://launchpad.net/~thepeakiscold/+archive/ubuntu/horsepower/+packages
```

A successful build appears in the PPA within 10–20 minutes. Failures are
reported by email with the full build log.

## Local build, no upload

```sh
python3 scripts/build-deb.py        # dist/horsepower_1.0.0_amd64.deb
sudo apt install ./dist/horsepower_1.0.0_amd64.deb
```

## Why the source tarball is 19 MB

Launchpad builders have no network, and `cargo build` wants crates.io. The
source package therefore carries `rust/horsepowerd/vendor/` (~120 MB extracted,
19 MB compressed), and `scripts/hpbuild.py` builds with `--offline`. That is
what `scripts/vendor-rust.py` prepares, and `scripts/make-source.py` runs it
automatically when the directory is missing.

If you would rather not ship the vendor tree, the alternative is to add the
crates as Debian packages (`librust-zbus-dev`, …). That is several days of
packaging work for a single application, which is why vendoring is the usual
choice for a PPA.

## Supported Ubuntu releases

| Release | `rustc` | Builds? |
| --- | --- | --- |
| 26.04 resolute | 1.93 | yes |
| 24.04 noble | 1.75 | **no** — the crate needs 1.89 |

To add 24.04 you would need a newer Rust than the archive ships, either from
`ppa:ubuntu-toolchain-r/test` or by lowering the dependency versions in
`rust/horsepowerd/Cargo.toml`. Neither is done yet; the changelog targets
`resolute` for that reason.

## Installing from the PPA

```sh
sudo add-apt-repository ppa:thepeakiscold/horsepower
sudo apt update
sudo apt install horsepower
```

Then start the service once if you do not want to wait for a reboot:

```sh
sudo systemctl enable --now horsepower.service
```

Uninstalling:

```sh
sudo apt remove horsepower      # keeps /var/lib/horsepower/power.db
sudo apt purge horsepower       # removes the log too
```
