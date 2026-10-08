# verso-plugin-ddns

Dynamic DNS for [Verso](https://github.com/we-are-mono/verso), the web
interface for OpenWrt: a name kept pointing at the router whenever its
connection gets a new address, over
[ddns-scripts](https://openwrt.org/docs/guide-user/base-system/ddns).

It is a non-core Verso plugin: a package of its own, installed only where you
want it. Once installed its page joins Verso's sidebar under Network; removed,
it leaves it. Like every Verso plugin it runs as its own process, describes its
page with Verso's widgets, and reads and writes the router only through rpcd
and the access list it ships.

Each `config service` section in `/etc/config/ddns` is one row. Its drawer asks
for a provider (Cloudflare, DuckDNS, No-IP, deSEC, dynv6, or any provider's
update URL), the name, and its secret, and writes the options ddns-scripts reads
for that provider. A row's state is what Verso's helper reads from the updater's
run folder (`/var/run/ddns`), never its log.

## Install

On a router running Verso, from the Verso package feed:

```sh
apk update
apk add verso-plugin-ddns
```

The package depends on `verso` 0.3.0 or newer, `ddns-scripts`,
`ddns-scripts-cloudflare` and `ddns-scripts-noip`.

## Build

The plugin is a static Rust binary built on Verso's plugin SDK, which Cargo
fetches from the Verso repository at the release this plugin pins.
`rust-toolchain.toml` provisions the compiler and both musl targets; nothing
else is needed but `rustup` and `make`.

```sh
make test     # unit and page tests
make lint     # clippy, warnings as errors
make build    # build/verso-plugin-ddns-{amd64,arm64}
make apk      # a signed package for the router (needs an OpenWrt buildroot)
```

`make apk` takes the apk tool and the signing key from an OpenWrt buildroot:
pass `OPENWRT_DIR=/path/to/openwrt/source`, or set it in a gitignored
`local.mk`. The version is the latest `vX.Y.Z` tag and the commits on it
(`make version`).

## Layout

- `src/` — the plugin: its page, the router reads behind it, and tests
- `manifest.json` — what the shell discovers: the page and its socket
- `i18n/` — its translations, one catalog per language
- `rootfs/` — what the package installs as it stands: the init script and the
  rpcd access list
- `packaging/` — the hooks that start the plugin's service and clean up after it

## License

GPL-2.0-only. See `LICENSE`.
