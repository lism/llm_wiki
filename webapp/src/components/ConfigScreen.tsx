import { useEffect, useState } from "react"
import { useStore } from "../store"
import { configure, healthCheck, listProjects, type Project } from "../api"
import { Sun, Moon, Wrench } from "lucide-react"

export function ConfigScreen() {
  const config = useStore(s => s.config)
  const setConfig = useStore(s => s.setConfig)
  const dark = useStore(s => s.dark)
  const toggleDark = useStore(s => s.toggleDark)

  const [url, setUrl] = useState(config.baseUrl)
  const [token, setToken] = useState(config.token)
  const [projectId, setProjectId] = useState(config.projectId)
  const [projects, setProjects] = useState<Project[]>([])
  const [status, setStatus] = useState<string>("")
  const [testing, setTesting] = useState(false)

  useEffect(() => {
    // Try loading projects on mount if already configured
    if (config.configured) {
      configure(config.baseUrl, config.token)
      loadProjects()
    }
  }, [])

  async function loadProjects() {
    try {
      const list = await listProjects()
      setProjects(list)
      // Auto-select current project
      const current = list.find(p => p.current)
      if (current && !projectId) setProjectId(current.id)
    } catch {
      // ignore
    }
  }

  async function testConnection() {
    setTesting(true)
    setStatus("Testing...")
    configure(url, token)
    try {
      const health = await healthCheck()
      if (!health.ok) {
        setStatus(`Server error: ${health.status}`)
        return
      }
      const list = await listProjects()
      setProjects(list)
      setStatus(`Connected — ${list.length} project(s) found`)
      const current = list.find(p => p.current)
      if (current) setProjectId(current.id)
    } catch (e: any) {
      setStatus(`Connection failed: ${e.message}`)
    } finally {
      setTesting(false)
    }
  }

  function saveAndEnter() {
    if (!projectId) return
    configure(url, token)
    setConfig({ baseUrl: url, token, projectId, configured: true })
  }

  return (
    <div className="min-h-screen flex items-center justify-center bg-zinc-50 dark:bg-zinc-900 p-4">
      <div className="w-full max-w-md space-y-6">
        <div className="text-center">
          <h1 className="text-2xl font-bold">LLM Wiki Chat</h1>
          <p className="text-sm text-zinc-500 mt-1">Connect to your wiki API server</p>
        </div>

        <div className="bg-white dark:bg-zinc-800 rounded-xl shadow-sm border border-zinc-200 dark:border-zinc-700 p-6 space-y-4">
          {/* API URL */}
          <div>
            <label className="block text-sm font-medium mb-1">API Server URL</label>
            <input
              type="text"
              value={url}
              onChange={e => setUrl(e.target.value)}
              placeholder="http://127.0.0.1:19828"
              className="w-full px-3 py-2 rounded-lg border border-zinc-300 dark:border-zinc-600 bg-white dark:bg-zinc-900 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
            />
          </div>

          {/* Token */}
          <div>
            <label className="block text-sm font-medium mb-1">API Token</label>
            <input
              type="password"
              value={token}
              onChange={e => setToken(e.target.value)}
              placeholder="Enter your API token"
              className="w-full px-3 py-2 rounded-lg border border-zinc-300 dark:border-zinc-600 bg-white dark:bg-zinc-900 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
            />
          </div>

          {/* Test + Status */}
          <div className="flex gap-2">
            <button
              onClick={testConnection}
              disabled={testing}
              className="px-4 py-2 text-sm rounded-lg border border-zinc-300 dark:border-zinc-600 hover:bg-zinc-100 dark:hover:bg-zinc-700 disabled:opacity-50"
            >
              {testing ? "Testing..." : "Test Connection"}
            </button>
            <button
              onClick={loadProjects}
              className="px-4 py-2 text-sm rounded-lg border border-zinc-300 dark:border-zinc-600 hover:bg-zinc-100 dark:hover:bg-zinc-700"
            >
              Refresh Projects
            </button>
          </div>
          {status && (
            <p className={`text-xs ${status.includes("failed") || status.includes("error") ? "text-red-500" : "text-green-600"}`}>
              {status}
            </p>
          )}

          {/* Project selector */}
          {projects.length > 0 && (
            <div>
              <label className="block text-sm font-medium mb-1">Project</label>
              <select
                value={projectId}
                onChange={e => setProjectId(e.target.value)}
                className="w-full px-3 py-2 rounded-lg border border-zinc-300 dark:border-zinc-600 bg-white dark:bg-zinc-900 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
              >
                <option value="">— Select a project —</option>
                {projects.map(p => (
                  <option key={p.id} value={p.id}>
                    {p.name} {p.current ? "(current)" : ""}
                  </option>
                ))}
              </select>
            </div>
          )}

          {/* Enter */}
          <button
            onClick={saveAndEnter}
            disabled={!projectId}
            className="w-full py-2.5 rounded-lg bg-blue-600 hover:bg-blue-700 text-white font-medium text-sm disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
          >
            Enter Chat
          </button>
        </div>

        {/* Dark mode toggle */}
        <div className="text-center">
          <button
            onClick={toggleDark}
            className="inline-flex items-center gap-1.5 text-xs text-zinc-400 hover:text-zinc-600 dark:hover:text-zinc-300"
          >
            {dark ? <Sun size={14} /> : <Moon size={14} />}
            {dark ? "Light" : "Dark"} mode
          </button>
        </div>
      </div>
    </div>
  )
}
