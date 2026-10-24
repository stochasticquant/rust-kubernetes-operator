## What and why

<!-- What does this change do, and why is it needed? Link the issue: "Closes #123". -->

## How it was tested

<!-- Unit tests, a kind/cluster run, manual steps. -->

## Checklist

- [ ] `cargo fmt`, `cargo clippy -- -D warnings`, `cargo test` and `cargo deny check` pass locally
- [ ] Tests added or updated for the change
- [ ] CRDs regenerated (`cargo run -p ferrogate -- crd > config/crd/...`) if `ferrogate-crd` changed
- [ ] Docs and `CHANGELOG.md` updated for user-visible changes
- [ ] Commits follow Conventional Commits and are signed off (`git commit -s`)

## Security impact

<!-- Does this touch the webhook, TLS, RBAC, the chart, CI, or what gets logged? If yes, explain. -->

- [ ] No security impact
- [ ] Security-relevant (explained above)
