// LLM Wiki API client — fetch + SSE streaming

export interface Project {
  id: string; name: string; path: string; current: boolean
}
export interface Reference {
  title: string; path: string; snippet: string
}
export interface ChatResponse {
  ok: boolean; answer?: string; error?: string; projectId?: string; references?: Reference[]
}
export interface SSECallbacks {
  onToken: (token: string) => void
  onReferences: (refs: Reference[]) => void
  onDone: () => void
  onError: (error: string) => void
}

let baseUrl = "http://127.0.0.1:19828"
let token = ""

export function configure(url: string, t: string) {
  baseUrl = url.replace(/\/+$/, ""); token = t
}
export function getBaseUrl() { return baseUrl }
export function getToken() { return token }

function authHeaders(): Record<string, string> {
  const h: Record<string, string> = { "Content-Type": "application/json" }
  if (token) h["Authorization"] = `Bearer ${token}`
  return h
}

export async function healthCheck() {
  return (await fetch(`${baseUrl}/api/v1/health`)).json()
}

export async function listProjects(): Promise<Project[]> {
  const r = await fetch(`${baseUrl}/api/v1/projects`, { headers: authHeaders() })
  const d = await r.json()
  if (!d.ok) throw new Error(d.error || "list projects failed")
  return d.projects || []
}

// ── SSE streaming ────────────────────────────────────────────────

export function chatStreaming(
  projectId: string,
  query: string,
  cb: SSECallbacks,
): AbortController {
  const ctrl = new AbortController()

  fetch(`${baseUrl}/api/v1/projects/${projectId}/chat`, {
    method: "POST",
    headers: authHeaders(),
    body: JSON.stringify({ query, stream: true }),
    signal: ctrl.signal,
  })
    .then(async (resp) => {
      if (!resp.ok) {
        const t = await resp.text()
        return cb.onError(`HTTP ${resp.status}: ${t.slice(0, 200)}`)
      }
      if (!resp.body) {
        return cb.onError("Response has no body — SSE requires HTTP streaming")
      }

      // Simple line-by-line reader
      const reader = resp.body.getReader()
      const dec = new TextDecoder()
      let buf = ""
      let ev = ""
      let parts: string[] = []

      function emit() {
        if (!ev) return
        const raw = parts.join("")
        parts = []; const name = ev; ev = ""
        try {
          const v = JSON.parse(raw)
          if (name === "token" && typeof v === "string") cb.onToken(v)
          else if (name === "references") cb.onReferences(v as Reference[])
          else if (name === "done") { cb.onDone(); return true }
          else if (name === "error") { cb.onError(typeof v === "object" ? (v as any).error : String(v)); return true }
        } catch { /* skip malformed frame */ }
        return false
      }

      try {
        while (true) {
          const { done, value } = await reader.read()
          if (done) break
          buf += dec.decode(value, { stream: true })

          // Process complete lines
          let nl: number
          while ((nl = buf.indexOf("\n")) !== -1) {
            const line = buf.slice(0, nl).trim()
            buf = buf.slice(nl + 1)

            if (line.startsWith("event: ")) {
              ev = line.slice(7)
            } else if (line.startsWith("data: ")) {
              parts.push(line.slice(6))
            } else if (line === "" && ev) {
              if (emit()) return  // done/error
            }
          }
        }
        // Stream ended — flush and complete
        if (ev) emit()
        cb.onDone()
      } catch (e: any) {
        if (e.name !== "AbortError") cb.onError(e.message || "Stream error")
      }
    })
    .catch((e) => {
      if (e.name !== "AbortError") cb.onError(e.message || "Network error")
    })

  return ctrl
}
