# Manifest equivalence tests

`manifests.rs` loads each checked-in example through the public config loader
and validator, then pins the seat identities, roles, native scopes, MCP
allowlists, iteration and sampling limits, workspace context and cross-hive
narrowing that the migrated binaries depend on. The executable offline proofs
also run actual provider requests and native tools.
