# GTK3 security backports

UntuShot still uses gtk-rs GTK3 0.18. Upgrading GLib alone to 0.20 is not
compatible with that stack. These three local Cargo patches keep the original
upstream versions and licenses; they are not upstream releases or a maintained
fork of the entire GTK3 stack. A future GTK4 migration should remove them.

## Sources and changes

The starting point is the published crates.io source of `glib 0.18.5`,
`glib-macros 0.18.5`, and `gtk3-macros 0.18.0`. Original package checksums and
hashes of changed/new files are recorded in `backports.json`. The reviewable
diff against those published sources is `gtk3-security.patch`. LICENSE,
COPYRIGHT and upstream attribution remain in the respective directories.
`Cargo.toml.orig` records the original upstream manifest; Cargo uses the patched
`Cargo.toml` files.

* **GLib:** backport the official [VariantStrIter fix, gtk-rs PR 1343](https://github.com/gtk-rs/gtk-rs-core/pull/1343):
  declare the output pointer mutable and pass `&mut p` to the variadic C call.
  This fixes [RUSTSEC-2024-0429](https://rustsec.org/advisories/RUSTSEC-2024-0429.html),
  also known as GHSA-wrw7-89jp-8q8g.
* **Macro crates:** remove `proc-macro-error` and its panic-based error adapter.
  Existing `syn::Error` facilities now produce compiler diagnostics directly;
  closure/variant parsers propagate `syn::Result`, including both source spans
  for duplicate `@watch`. No replacement error-handling dependency is needed.
  Both [the original crate](https://rustsec.org/advisories/RUSTSEC-2024-0370.html)
  and [proc-macro-error2](https://rustsec.org/advisories/RUSTSEC-2026-0173.html)
  are unmaintained. Valid macro generation is preserved.

The main lockfile also updates `anyhow` to at least 1.0.103 and `png` to 0.17.16;
this fixes RUSTSEC-2026-0190 and removes the old `adler` dependency.

## Verification and scanner interpretation

`make audit` verifies patched file hashes and Cargo's resolved source paths,
runs the security regression tests **with optimization**, then runs OSV against
Cargo.lock. The tests cover every affected VariantStrIter iterator operation,
the anyhow mutable-context case, valid macro expansion and compiler diagnostics
for malformed macro input. Normal application tests and the GTK smoke test cover
the compiled application as well.

OSV sees the unchanged GLib version 0.18.5 and cannot recognize the patched
source. The root `osv-scanner.toml` therefore contains **one narrowly scoped
exception for the fixed GLib advisory and its alias**, expiring 2027-01-31 for
review. Other GLib advisories and all other packages remain checked. There are
no exceptions for unresolved security defects or maintenance warnings.

To see the unfiltered version-based result independently:

```sh
printf '' > /tmp/untushot-empty-osv.toml
osv-scanner scan source --config /tmp/untushot-empty-osv.toml --lockfile Cargo.lock
```

That scan will still report the backported GLib advisory. A successful configured
scan is not a claim that upstream GLib 0.18.5 was fixed or that GTK3 is fully
maintained. Recheck the patch, lockfile and tests before updating this inventory
or renewing the exception. OSV covers known Rust-package advisories here, not
Ubuntu's native GTK/GLib/image-loader packages.
