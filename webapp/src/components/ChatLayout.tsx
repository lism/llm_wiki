import { useState } from "react"
import { useStore } from "../store"
import { ConversationList } from "./ConversationList"
import { ChatPanel } from "./ChatPanel"
import { Menu, X } from "lucide-react"

export function ChatLayout() {
  const [sidebarOpen, setSidebarOpen] = useState(false)

  return (
    <div className="h-screen flex">
      {/* Mobile overlay */}
      {sidebarOpen && (
        <div
          className="fixed inset-0 bg-black/40 z-40 lg:hidden"
          onClick={() => setSidebarOpen(false)}
        />
      )}

      {/* Sidebar */}
      <div
        className={`
          fixed lg:static inset-y-0 left-0 z-50 w-64
          bg-white dark:bg-zinc-900 border-r border-zinc-200 dark:border-zinc-800
          transform transition-transform lg:translate-x-0
          ${sidebarOpen ? "translate-x-0" : "-translate-x-full"}
        `}
      >
        <ConversationList onSelect={() => setSidebarOpen(false)} />
      </div>

      {/* Main area */}
      <div className="flex-1 flex flex-col min-w-0">
        {/* Top bar (mobile) */}
        <div className="lg:hidden flex items-center gap-3 px-4 py-2 border-b border-zinc-200 dark:border-zinc-800">
          <button
            onClick={() => setSidebarOpen(true)}
            className="p-1 hover:bg-zinc-100 dark:hover:bg-zinc-800 rounded"
          >
            <Menu size={20} />
          </button>
          <span className="font-semibold text-sm">LLM Wiki Chat</span>
        </div>

        <ChatPanel />
      </div>
    </div>
  )
}
