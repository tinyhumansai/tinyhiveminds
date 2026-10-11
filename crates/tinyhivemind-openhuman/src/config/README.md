# Manifest configuration

`HiveConfig::from_json` and `from_value` parse canonical JSON without opening
files or resolving credentials. `load_dir` explicitly reads `hive.json`,
`profiles/*.md` and `context/*.md` relative to its supplied root. YAML profile
frontmatter configures the typed profile; its Markdown body supplies the prompt.
No parser error includes input contents.

Call `validate` before constructing runtime objects. `ValidatedHiveConfig`
exposes immutable data through `config()`; consuming it with `into_config()`
returns editable data without retaining validation proof. See the
[accepted contract](../../../../docs/specs/openhuman-hive-config.md).

| File | Responsibility |
| --- | --- |
| `mod.rs` | Public configuration exports |
| `types.rs` | Canonical serde fields and immutable validated wrapper |
| `parse.rs` | JSON, relative Markdown/YAML loading, inline-secret refusal |
| `validate.rs` | References, identities, limits, narrowing and MCP consistency |
| `test/` | Wire, loader and validation regression tests |

Named `SecretRef` values encode as `{"env":"NAME"}` or `{"store":"KEY"}`.
MCP environment and header values require references. Seat overrides have no
provider credential field. Runtime-only ports and resolved secrets never enter
the manifest.

Permission layers compose as an intersection; omitted optional allowlists and
approval/rule fields inherit. Explicit allowlists are subsets. Denylists must
retain parent denials. Conservative rule equivalence requires identical explicit
argument-sensitive rules and approval policies, except explicit disabling of
approval authority. Deployment must enforce every layer, including inherited
ones. Runtime, profile and seat turn limits can only decrease.

`normalized_mcp` combines runtime/profile/seat declarations, sorts tool names,
and rejects conflicting named connections. Membership MCP declarations assert
the complete same set; use two seats to change connections.

Adapter hive defaults use episode `round_width=1`, `revealed_width=1`. Each
width is independently bounded by runtime concurrency; blind rounds may be
wider than revealed rounds. Core defaults remain unchanged. Other policies
retain core and coordinator defaults.

Directory loading ignores conventional `README.md` files, case-insensitively,
in both `profiles/` and `context/`. These document the directories without
creating an extra profile or context entry. Other Markdown names load normally.
