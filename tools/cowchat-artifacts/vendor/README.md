# Gallop 0.7.0 vendor provenance

These archives contain the actual published `@cowboyinc/gallop-components`
and `@cowboyinc/gallop-foundations` 0.7.0 packages. They were repacked without
code changes from the registry-installed packages using `npm pack
--ignore-scripts`; `dist/` code, types, source maps, CSS, and package metadata
are retained. Both package manifests declare `license: MIT`. Upstream did not
include a separate license file; see `LICENSE-MIT.txt` for attribution and MIT
terms. Source: https://github.com/cowboyinc/gallop (`packages/components` and
`packages/foundations`). No private registry access is needed to install/build
this repository.

Original published archive references recorded in the previous lockfile:

- components: `https://npm.pkg.github.com/download/@cowboyinc/gallop-components/0.7.0/50701d47383c7b3b430ed594fa84c425812d7cfa`, integrity `sha512-XSGnPfPfsLri6pN8nTiqVpykT0plPDCwlx7Si2UC4qHBTAdpAFgzq4QXFtuIZY3wO72eGTeCjCRi4tP0fp8pOQ==`
- foundations: `https://npm.pkg.github.com/download/@cowboyinc/gallop-foundations/0.7.0/7d08623d46068675621a72b15f0d98c18947bdf2`, integrity `sha512-mNPvU00jU/Q4hOs3utl3OTdZXnKIqG+Qu22JkXoaK8066qRucUGf/9T4+KI3qpg/YpG+nwpKgAVPQSE9f5c2Mw==`

Repacked archive SHA-256:

```
5bc1c1d0da5e0ecd8a2e181f2f3af6bb73282546d29ef5031977295b057d92a0  cowboyinc-gallop-components-0.7.0.tgz
231c090f595fbe4e3ce048f801fd573eaad9bed62be85cfa38a8ae221650c9ec  cowboyinc-gallop-foundations-0.7.0.tgz
```

To reproduce these archives from this checkout, install with the frozen
lockfile, then run from `tools/cowchat-artifacts`:

```
npm pack --ignore-scripts ./node_modules/@cowboyinc/gallop-components --pack-destination ./vendor
npm pack --ignore-scripts ./node_modules/@cowboyinc/gallop-foundations --pack-destination ./vendor
shasum -a 256 vendor/*.tgz
```

To refresh to a new release, obtain its published packages using an authorized
upstream checkout or registry session outside this repository. Verify their
version, license and source, repack with the same commands, update the relative
`file:vendor/` dependency pins and this provenance record, and regenerate the
lockfile. Verify installation with empty npm configuration and a fresh package
store, then rebuild and run the browser tests. Never add registry credentials
to this repository.
