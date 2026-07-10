#!/usr/bin/env python3
"""Test script for LLM Wiki local HTTP API chat endpoint.

Usage:
    python test_chat_api.py                          # run all tests
    python test_chat_api.py --base http://localhost:19828  # custom base URL
    python test_chat_api.py --token YOUR_TOKEN        # auth token
    python test_chat_api.py --skip-stream             # skip streaming test
    python test_chat_api.py --query "what is this?"   # custom query

Run without arguments to use defaults:
    base  = http://127.0.0.1:19828
    token = GFPoWuK7kpqepXJ-m8N1gb2HeQArqvDePA1tEXUQW-I
"""

import argparse
import json
import sys
import time
from http.client import HTTPConnection, HTTPException
from urllib.parse import urlparse


class TestResult:
    def __init__(self, name: str):
        self.name = name
        self.passed: bool | None = None
        self.message = ""
        self.duration_ms = 0

    def ok(self, msg: str = "") -> None:
        self.passed = True
        self.message = msg

    def fail(self, msg: str) -> None:
        self.passed = False
        self.message = msg

    def status(self) -> str:
        if self.passed is None:
            return "SKIP"
        return "PASS" if self.passed else "FAIL"


class ChatApiTester:
    def __init__(self, base_url: str, token: str):
        parsed = urlparse(base_url)
        self.host = parsed.hostname or "127.0.0.1"
        self.port = parsed.port or 19829
        self.token = token
        self.results: list[TestResult] = []

    def _request(self, method: str, path: str, body: str | None = None) -> tuple[int, dict]:
        """Send an HTTP request and return (status_code, parsed_json_body)."""
        conn = HTTPConnection(self.host, self.port, timeout=30)
        headers = {
            "Content-Type": "application/json",
            "Authorization": f"Bearer {self.token}",
        }
        try:
            conn.request(method, path, body=body, headers=headers)
            resp = conn.getresponse()
            data = resp.read().decode("utf-8")
            try:
                parsed = json.loads(data)
            except json.JSONDecodeError:
                parsed = {"_raw": data}
            return resp.status, parsed
        finally:
            conn.close()

    def _stream_request(self, path: str, body: str) -> list[dict]:
        """Send a streaming request and collect all SSE events."""
        conn = HTTPConnection(self.host, self.port, timeout=120)
        headers = {
            "Content-Type": "application/json",
            "Authorization": f"Bearer {self.token}",
        }
        events: list[dict] = []
        try:
            conn.request("POST", path, body=body, headers=headers)
            resp = conn.getresponse()
            if resp.status != 200:
                data = resp.read().decode("utf-8")
                try:
                    events.append({"error": json.loads(data)})
                except json.JSONDecodeError:
                    events.append({"error": data})
                return events

            current_event: str | None = None
            current_data: list[str] = []
            for line_bytes in resp:
                line = line_bytes.decode("utf-8").rstrip("\n").rstrip("\r")
                if line.startswith("event: "):
                    current_event = line[7:]
                elif line.startswith("data: "):
                    current_data.append(line[6:])
                elif line == "" and current_event is not None:
                    raw = "".join(current_data)
                    try:
                        parsed = json.loads(raw)
                    except json.JSONDecodeError:
                        parsed = raw
                    events.append({"event": current_event, "data": parsed})
                    current_event = None
                    current_data = []
        finally:
            conn.close()
        return events

    def test(self, name: str):
        """Decorator-like helper: create and register a test."""
        r = TestResult(name)
        self.results.append(r)
        return r

    # ── tests ──────────────────────────────────────────────────────

    def run_health(self):
        r = self.test("GET /api/v1/health")
        t0 = time.monotonic()
        try:
            status, data = self._request("GET", "/api/v1/health")
            if status == 200 and data.get("ok") and data.get("status") == "running":
                r.ok(f"status={data['status']}, version={data.get('version')}")
            else:
                r.fail(f"status={status}, body={json.dumps(data)[:200]}")
        except HTTPException as e:
            r.fail(str(e))
        r.duration_ms = (time.monotonic() - t0) * 1000

    def run_list_projects(self):
        r = self.test("GET /api/v1/projects")
        t0 = time.monotonic()
        try:
            status, data = self._request("GET", "/api/v1/projects")
            if status == 200 and data.get("ok"):
                projects = data.get("projects", [])
                current = data.get("currentProject")
                r.ok(f"{len(projects)} projects, current='{current.get('name','?')}'")
                # store for later tests
                self._project_id = current.get("id") if current else None
            else:
                r.fail(f"status={status}, body={json.dumps(data)[:200]}")
        except HTTPException as e:
            r.fail(str(e))
        r.duration_ms = (time.monotonic() - t0) * 1000

    def run_chat_non_streaming(self, project_id: str, query: str):
        r = self.test(f"POST /chat (non-streaming): {query[:50]}")
        t0 = time.monotonic()
        try:
            body = json.dumps({"query": query, "stream": False})
            status, data = self._request(
                "POST", f"/api/v1/projects/{project_id}/chat", body
            )
            if status != 200:
                r.fail(f"HTTP {status}: {json.dumps(data)[:200]}")
                return
            answer = data.get("answer", "")
            refs = data.get("references", [])
            if answer:
                cited = "<!-- cited:" in answer
                wikilinks = "[[" in answer
                details = []
                if cited:
                    details.append("cited")
                if wikilinks:
                    details.append("wikilinks")
                if refs:
                    details.append(f"{len(refs)} refs")
                r.ok(f"{len(answer)} chars, {', '.join(details) if details else 'ok'}")
            else:
                r.fail("empty answer")
        except HTTPException as e:
            r.fail(str(e))
        r.duration_ms = (time.monotonic() - t0) * 1000

    def run_chat_streaming(self, project_id: str, query: str):
        r = self.test(f"POST /chat (SSE streaming): {query[:50]}")
        t0 = time.monotonic()
        try:
            body = json.dumps({"query": query, "stream": True})
            events = self._stream_request(
                f"/api/v1/projects/{project_id}/chat", body
            )
            if not events:
                r.fail("no SSE events received")
                return
            if "error" in events[0]:
                r.fail(f"error event: {events[0]['error']}")
                return
            tokens = [e for e in events if e.get("event") == "token"]
            refs_event = [e for e in events if e.get("event") == "references"]
            done = [e for e in events if e.get("event") == "done"]
            ref_count = len(refs_event[0]["data"]) if refs_event else 0
            r.ok(f"{len(tokens)} token events, {ref_count} refs, done={len(done) > 0}")
        except HTTPException as e:
            r.fail(str(e))
        r.duration_ms = (time.monotonic() - t0) * 1000

    def run_auth_required(self):
        r = self.test("POST /chat without token -> 401")
        t0 = time.monotonic()
        try:
            conn = HTTPConnection(self.host, self.port, timeout=10)
            conn.request(
                "POST",
                "/api/v1/projects/nonexistent/chat",
                body=json.dumps({"query": "test", "stream": False}),
                headers={"Content-Type": "application/json"},
            )
            resp = conn.getresponse()
            conn.close()
            if resp.status == 401:
                r.ok()
            else:
                r.fail(f"expected 401, got {resp.status}")
        except HTTPException as e:
            r.fail(str(e))
        r.duration_ms = (time.monotonic() - t0) * 1000

    def run_missing_query(self, project_id: str):
        r = self.test("POST /chat empty query -> 400")
        t0 = time.monotonic()
        try:
            body = json.dumps({"query": "", "stream": False})
            status, data = self._request(
                "POST", f"/api/v1/projects/{project_id}/chat", body
            )
            if status == 400:
                r.ok(f"error: {data.get('error', '')[:80]}")
            else:
                r.fail(f"expected 400, got {status}")
        except HTTPException as e:
            r.fail(str(e))
        r.duration_ms = (time.monotonic() - t0) * 1000

    def run_unknown_project(self):
        r = self.test("POST /chat unknown project -> 404")
        t0 = time.monotonic()
        try:
            body = json.dumps({"query": "test", "stream": False})
            status, data = self._request(
                "POST", "/api/v1/projects/nonexistent-id/chat", body
            )
            if status == 404:
                r.ok()
            else:
                r.fail(f"expected 404, got {status}: {json.dumps(data)[:100]}")
        except HTTPException as e:
            r.fail(str(e))
        r.duration_ms = (time.monotonic() - t0) * 1000

    def print_summary(self):
        passed = sum(1 for r in self.results if r.passed is True)
        failed = sum(1 for r in self.results if r.passed is False)
        skipped = sum(1 for r in self.results if r.passed is None)

        print()
        print("=" * 60)
        print(f"  Results: {passed} passed, {failed} failed, {skipped} skipped")
        print("=" * 60)
        for r in self.results:
            tag = r.status()
            color = {"PASS": "\033[92m", "FAIL": "\033[91m", "SKIP": "\033[90m"}.get(
                tag, ""
            )
            reset = "\033[0m" if color else ""
            dur = f" ({r.duration_ms:.0f}ms)" if r.duration_ms > 0 else ""
            print(f"  {color}[{tag}]{reset} {r.name}{dur}")
            if r.message:
                print(f"       {r.message}")
        print("=" * 60)
        return failed == 0


def main():
    parser = argparse.ArgumentParser(
        description="LLM Wiki Chat API test script",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=__doc__,
    )
    parser.add_argument(
        "--base",
        default="http://127.0.0.1:19828",
        help="API base URL (default: http://127.0.0.1:19828)",
    )
    parser.add_argument(
        "--token",
        default="GFPoWuK7kpqepXJ-m8N1gb2HeQArqvDePA1tEXUQW-I",
        help="API auth token",
    )
    parser.add_argument(
        "--query",
        default="what is this wiki about? summarize in one sentence.",
        help="Test query (default: a discovery question)",
    )
    parser.add_argument(
        "--skip-stream",
        action="store_true",
        help="Skip the SSE streaming test (saves time/API costs)",
    )
    args = parser.parse_args()

    tester = ChatApiTester(args.base, args.token)

    # 1. Health check
    tester.run_health()

    # 2. List projects
    tester.run_list_projects()
    project_id = getattr(tester, "_project_id", None)

    if not project_id:
        print("\n[!] No current project found — skipping chat tests.")
        tester.print_summary()
        sys.exit(1)

    # 3. Error cases
    tester.run_auth_required()
    tester.run_missing_query(project_id)
    tester.run_unknown_project()

    # 4. Non-streaming chat
    tester.run_chat_non_streaming(project_id, args.query)

    # 5. SSE streaming chat
    if not args.skip_stream:
        tester.run_chat_streaming(project_id, args.query)

    ok = tester.print_summary()
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
