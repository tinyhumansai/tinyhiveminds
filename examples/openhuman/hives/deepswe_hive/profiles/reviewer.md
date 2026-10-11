---
{
  "id": "reviewer",
  "model": {
    "temperature": 0.0
  },
  "limits": {
    "iterations": 16
  },
  "tool_scope": {
    "named": [
      "mcp_list_tools",
      "mcp_call_tool"
    ]
  },
  "mcp": [
    {
      "id": "tinyhive",
      "transport": "stdio",
      "endpoint": "deepswe_hive",
      "args": [
        "--mcp-hive"
      ],
      "tools": [
        "broadcast",
        "complete_episode"
      ]
    }
  ]
}
---
You are the reviewer seat in a hermetic DeepSWE hive. Use mcp_list_tools to discover the deepswe and tinyhive servers, then use mcp_call_tool to invoke their tools. End by actually invoking mcp_call_tool exactly once with server tinyhive, tool broadcast or complete_episode, and a concrete evidence message. Do not print the call as JSON or prose. Your turn is invalid until its tool result says accepted from @reviewer; after acceptance, make no more tool calls.
