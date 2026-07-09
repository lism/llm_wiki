"""
LLM Wiki Chat API demo — asks a question and prints the streaming answer.

Usage:
    python chat_demo.py "你的问题"
    python chat_demo.py "这个wiki讲了什么？"
"""

import json, sys
from http.client import HTTPConnection

BASE = "127.0.0.1:19828"
TOKEN = "GFPoWuK7kpqepXJ-m8N1gb2HeQArqvDePA1tEXUQW-I"
PROJECT = "current"  # or use a specific project id

query = sys.argv[1] if len(sys.argv) > 1 else "what is this wiki about?"

# ── POST /chat (SSE streaming) ──
conn = HTTPConnection(BASE, timeout=120)
body = json.dumps({"query": query, "stream": True})
headers = {
    "Content-Type": "application/json",
    "Authorization": f"Bearer {TOKEN}",
}
conn.request("POST", f"/api/v1/projects/{PROJECT}/chat", body=body, headers=headers)
resp = conn.getresponse()

if resp.status != 200:
    print(f"Error {resp.status}: {resp.read().decode()}")
    sys.exit(1)

event = None
data_parts = []

for line_bytes in resp:
    line = line_bytes.decode("utf-8").strip()
    if line.startswith("event: "):
        event = line[7:]
    elif line.startswith("data: "):
        data_parts.append(line[6:])
    elif line == "" and event is not None:
        raw = "".join(data_parts)
        if event == "token":
            print(json.loads(raw), end="", flush=True)
        elif event == "references":
            refs = json.loads(raw)
            if refs:
                print(f"\n\n--- 引用 ---")
                for r in refs:
                    print(f"  [{r['title']}] {r['path']}")
        elif event == "done":
            print("\n")
            break
        elif event == "error":
            print(f"\n错误: {json.loads(raw)}")
            break
        event = None
        data_parts = []

conn.close()
