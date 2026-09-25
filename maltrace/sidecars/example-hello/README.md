# example-hello

The smallest possible maltrace plugin — ~40 lines of pure-stdlib Python.

Use this as a copy-paste starting point. Copy [hello.py](hello.py) and
[plugin.toml](plugin.toml) into a new directory under
`maltrace/sidecars/your-plugin/`, change the `id`, and you have a working
plugin.

See [docs/PLUGIN_SDK.md](../../docs/PLUGIN_SDK.md) for the full plugin spec.

## What it does

One transform, `example_hello`, that takes a `DOMAIN_NAME` (or
`INTERNET_NAME`) and emits one `DESCRIPTION_ABSTRACT` event saying
`hello, <domain>`. Useful only as a contract demonstration.

## Try it standalone

```bash
cd maltrace/sidecars/example-hello
python3 hello.py list-transforms
python3 hello.py run --module example_hello --event-type DOMAIN_NAME --data example.com
```

## See it in the app

`SidecarRegistry::from_repo_layout()` scans `maltrace/sidecars/*/plugin.toml`
at startup. Launch the app, right-click a Domain entity, and `example_hello`
appears under "example-hello" in the transform menu.
