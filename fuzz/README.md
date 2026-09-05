The `document` target exercises parsing, page extraction, and rendering.
It limits input size and page count. The command also bounds execution time
per input and resident memory. Valid and cyclic page-tree seeds live in
`tests/fixtures/`, alongside deterministic mutation regressions.

```sh
cargo install cargo-fuzz --version 0.13.2 --locked
mkdir -p fuzz/corpus/document
cp tests/fixtures/*.pdf fuzz/corpus/document/
cargo +nightly fuzz run document -- -max_total_time=60 -timeout=5 -rss_limit_mb=512 -max_len=262144
```

CI runs a bounded campaign on relevant pull requests and weekly. Failures upload
the reproducing inputs. Minimize a failing input and add a deterministic test
before changing the parser. A successful bounded run is not proof that every
possible PDF is safe.
