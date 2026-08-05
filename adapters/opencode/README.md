# OpenCode adapter

Installable artifacts:

| File | Purpose |
|---|---|
| [`plugin.json`](./plugin.json) | Declarative command + hook map |
| [`inject.md`](./inject.md) | Pressure injection guidance |
| [`install.md`](./install.md) | Activation steps |

Adapters should check `protocol_version` in JSON responses and fail closed on unsupported versions.
