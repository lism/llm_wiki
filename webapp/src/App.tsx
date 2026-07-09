import { useEffect } from "react"
import { useStore } from "./store"
import { configure, healthCheck } from "./api"
import { ConfigScreen } from "./components/ConfigScreen"
import { ChatLayout } from "./components/ChatLayout"

export default function App() {
  const config = useStore(s => s.config)
  const setConfig = useStore(s => s.setConfig)
  const dark = useStore(s => s.dark)

  useEffect(() => {
    document.documentElement.classList.toggle("dark", dark)
  }, [dark])

  useEffect(() => {
    configure(config.baseUrl, config.token)
  }, [config.baseUrl, config.token])

  // Verify config on mount
  useEffect(() => {
    if (!config.configured) return
    healthCheck()
      .then(d => {
        if (!d.ok) setConfig({ configured: false })
      })
      .catch(() => setConfig({ configured: false }))
  }, [])

  if (!config.configured || !config.projectId) {
    return <ConfigScreen />
  }

  return <ChatLayout />
}
