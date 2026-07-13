"""
FastGPT API demo — streaming chat with SSE.

Usage:
    python fastgpt_demo.py "你的问题"
    python fastgpt_demo.py "什么是注意力机制？"
    python fastgpt_demo.py                     # 默认问题
"""

import json, sys
from http.client import HTTPConnection

BASE = "199.66.68.17:3100"
KEY = "YOUR_FASTGPT_API_KEY"
ENDPOINT = "/api/v1/chat/completions"

query = sys.argv[1] if len(sys.argv) > 1 else "hello, introduce yourself in one sentence"

body = json.dumps({
    "model": "gpt-3.5-turbo",
    "messages": [{"role": "user", "content": query}],
    "stream": True,
})

conn = HTTPConnection(BASE, timeout=120)
conn.request("POST", ENDPOINT, body=body, headers={
    "Content-Type": "application/json",
    "Authorization": f"Bearer {KEY}",
})

resp = conn.getresponse()
if resp.status != 200:
    print(f"Error {resp.status}: {resp.read().decode()}")
    sys.exit(1)

buf = ""
for chunk in resp:
    buf += chunk.decode("utf-8")
    while "\n" in buf:
        line, buf = buf.split("\n", 1)
        line = line.strip()
        if line.startswith("data: "):
            data = line[6:]
            if data == "[DONE]":
                print()
                break
            try:
                token = json.loads(data)["choices"][0]["delta"].get("content", "")
                print(token, end="", flush=True)
            except:
                pass

conn.close()
