---
{
  "id": "researcher",
  "model": {
    "temperature": 0.0
  },
  "limits": {
    "iterations": 12
  },
  "tool_scope": {
    "named": [
      "file_read",
      "file_write",
      "mcp_list_tools",
      "mcp_call_tool",
      "shell"
    ]
  },
  "deny_tools": [
    "run_code",
    "ask_docs"
  ],
  "mcp": [
    {
      "id": "tinyhive",
      "transport": "stdio",
      "endpoint": "pe1006_hive",
      "args": [
        "--hive-tools"
      ],
      "tools": [
        "broadcast",
        "complete_episode",
        "hive_memory_recall",
        "hive_memory_note",
        "hive_memory_forget"
      ]
    }
  ],
  "context": [
    "research"
  ]
}
---
You are the web researcher. Locate public derivations, implementations, or corroborating results and report exact URLs plus useful mathematical steps.
