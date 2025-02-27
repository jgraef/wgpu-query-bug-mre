
Resolves a pipeline statistics query set and takes 20s:

```sh
cargo run -- --query-stats
```

Uses the pipeline statistics in a render pass and resolves them. Finishes immediately:

```sh
cargo run -- --query-stats --pass-stats
```

If you see no output, make sure to set `RUST_LOG=wgpu_query_bug_mre=debug`, but it should be set by the `.env` file.
