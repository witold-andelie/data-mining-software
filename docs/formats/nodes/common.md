# Common node properties

Probed on the Type node (see `type.md`); the same elements appear for every node.

| scripting property | element (sibling of settings inside node `properties`) | encoding | default |
|---|---|---|---|
| `use_custom_name` | `autoLabel` | inverse flag: True -> `false` | `true` |
| `custom_name` | `label` | string | empty |
| `tooltip` | `toolTipText` | string | empty |
| `annotation` | `annotation` | string | empty |
| `keywords` | `keywords/value` | single string `"kw1 kw2"` (API reads `[kw1 kw2]`) | empty |
| `cache_enabled` | `cacheEnabled` | `true`/`false` | `false` |

The first property set on a node flips `autoLabel` to false in the saved file as a side effect.
