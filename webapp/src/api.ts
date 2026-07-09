// LLM Wiki API client — fetch + SSE streaming

export interface Project {
  id: string
  name: string
  path: string
  current: boolean
}

export interface Reference {
  title: string
  path: string
  snippet: string
}

export interface ChatResponse {
  ok: boolean
  answer?: string
  error?: string
  projectId?: string
  references?: Reference[]
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
  baseUrl = url.replace(/\/+$/, "")
  token = t
}

export function getBaseUrl() { return baseUrl }
export function getToken() { return token }

function headers(): Record<string, string> {
  const h: Record<string, string> = { "Content-Type": "application/json" }
  if (token) h["Authorization"] = `Bearer ${token}`
  return h
}

export async function healthCheck(): Promise<{ ok: boolean; status: string; version?: string }> {
  const resp = await fetch(`${baseUrl}/api/v1/health`)
  return resp.json()
}

export async function listProjects(): Promise<Project[]> {
  const resp = await fetch(`${baseUrl}/api/v1/projects`, { headers: headers() })
  const data = await resp.json()
  if (!data.ok) throw new Error(data.error || "Failed to list projects")
  return data.projects || []
}

export async function chatNonStreaming(
  projectId: string,
  query: string,
): Promise<ChatResponse> {
  const resp = await fetch(`${baseUrl}/api/v1/projects/${projectId}/chat`, {
    method: "POST",
    headers: headers(),
    body: JSON.stringify({ query, stream: false }),
  })
  return resp.json()
}

export function chatStreaming(
  projectId: string,
  query: string,
  callbacks: SSECallbacks,
): AbortController {
  const controller = new AbortController()

  fetch(`${baseUrl}/api/v1/projects/${projectId}/chat`, {
    method: "POST",
    headers: headers(),
    body: JSON.stringify({ query, stream: true }),
    signal: controller.signal,
  }).then(async (resp) => {
    if (!resp.ok || !resp.body) {
      const text = await resp.text()
      try {
        const err = JSON.parse(text)
        callbacks.onError(err.error || `HTTP ${resp.status}`)
      } catch {
        callbacks.onError(`HTTP ${resp.status}: ${text.slice(0, 200)}`)
      }
      return
    }

    const reader = resp.body.getReader()
    const decoder = new TextDecoder()
    let buffer = ""
    let currentEvent = ""
    let dataParts: string[] = []

    try {
      while (true) {
        const { done, value } = await reader.read()
        if (done) break
        buffer += decoder.decode(value, { stream: true })

        const lines = buffer.split("\n")
        buffer = lines.pop() || ""

        for (const line of lines) {
          const trimmed = line.trim()
          if (trimmed.startsWith("event: ")) {
            currentEvent = trimmed.slice(7)
          } else if (trimmed.startsWith("data: ")) {
            dataParts.push(trimmed.slice(6))
          } else if (trimmed === "" && currentEvent) {
            const raw = dataParts.join("")
            dataParts = []
            try {
              const parsed = JSON.parse(raw)
              switch (currentEvent) {
                case "token":
                  if (typeof parsed === "string") callbacks.onToken(parsed)
                  break
                case "references":
                  callbacks.onReferences(parsed as Reference[])
                  break
                case "done":
                  callbacks.onDone()
                  return
                case "error":
                  callbacks.onError(typeof parsed === "object" ? parsed.error : String(parsed))
                  return
              }
            } catch {
              // skip unparseable frames
            }
            currentEvent = ""
          }
        }
      }
    } catch (e: any) {
      if (e.name !== "AbortError") {
        callbacks.onError(e.message || "Stream read error")
      }
    }
  }).catch(e => {
    if (e.name !== "AbortError") {
      callbacks.onError(e.message || "Connection error")
    }
  })

  return controller
}
