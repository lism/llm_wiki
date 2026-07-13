"""
FastGPT API demo — streaming chat with SSE.

Usage:
    python fastgpt_demo.py "你的问题"
    python fastgpt_demo.py                     # 默认问题
    python fastgpt_demo.py --debug "你的问题"   # 显示原始响应
    python fastgpt_demo.py --list-models       # 列出可用模型
"""

import json, sys
from http.client import HTTPConnection

BASE = "199.66.68.17:3100"
KEY = "YOUR_FASTGPT_API_KEY"
ENDPOINT = "/api/v1/chat/completions"
MODEL = "gpt-3.5-turbo"  # FastGPT 的模型名可能不同，可以 --list-models 查看

# ── helpers ──

def api_request(method, path, body=None):
    conn = HTTPConnection(BASE, timeout=30)
    headers = {"Content-Type": "application/json", "Authorization": f"Bearer {KEY}"}
    conn.request(method, path, body=body, headers=headers)
    resp = conn.getresponse()
    data = resp.read().decode("utf-8")
    conn.close()
    return resp.status, data

# ── list models ──

def list_models():
    status, data = api_request("GET", "/api/v1/models")
    print(f"HTTP {status}")
    try:
        models = json.loads(data)
        if "data" in models:
            for m in models["data"]:
                print(f"  {m['id']}")
        else:
            print(json.dumps(models, indent=2, ensure_ascii=False)[:2000])
    except:
        print(data[:1000])

# ── chat ──

def chat(query, debug=False):
    body = json.dumps({
        "model": MODEL,
        "messages": [{"role": "user", "content": query}],
        "stream": True,
    })

    conn = HTTPConnection(BASE, timeout=120)
    conn.request("POST", ENDPOINT, body=body, headers={
        "Content-Type": "application/json",
        "Authorization": f"Bearer {KEY}",
    })

    resp = conn.getresponse()
    if debug:
        print(f"Status: {resp.status}")
        print(f"Headers: {dict(resp.getheaders())}")
        print("---")

    if resp.status != 200:
        raw = resp.read().decode("utf-8")
        print(f"Error {resp.status}: {raw[:500]}")
        conn.close()
        return

    buf = ""
    for chunk in resp:
        buf += chunk.decode("utf-8")
        if debug:
            print(f"[RAW] {json.dumps(buf[-200:])}")
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
                    if debug:
                        print(f"\n[PARSE FAIL] {data[:200]}")
            elif line and debug:
                print(f"[LINE] {line[:200]}")
    conn.close()

# ── main ──

if __name__ == "__main__":
    args = sys.argv[1:]
    debug = "--debug" in args
    args = [a for a in args if a != "--debug"]

    if "--list-models" in args:
        list_models()
    else:
        query = args[0] if args else "say hello in one short sentence"
        chat(query, debug=debug)
