import { useState } from "react"
import { useStore } from "../store"
import { Plus, Trash2, MessageSquare, Sun, Moon, Settings } from "lucide-react"

interface Props {
  onSelect?: () => void
}

export function ConversationList({ onSelect }: Props) {
  const conversations = useStore(s => s.conversations)
  const activeId = useStore(s => s.activeConversationId)
  const createConversation = useStore(s => s.createConversation)
  const deleteConversation = useStore(s => s.deleteConversation)
  const setActive = useStore(s => s.setActiveConversation)
  const dark = useStore(s => s.dark)
  const toggleDark = useStore(s => s.toggleDark)
  const setConfig = useStore(s => s.setConfig)
  const messages = useStore(s => s.messages)
  const [hovered, setHovered] = useState<string | null>(null)

  const sorted = [...conversations].sort((a, b) => b.updatedAt - a.updatedAt)

  function handleSelect(id: string) {
    setActive(id)
    onSelect?.()
  }

  return (
    <div className="flex flex-col h-full">
      {/* New chat button */}
      <div className="p-3 border-b border-zinc-200 dark:border-zinc-800">
        <button
          onClick={() => { const id = createConversation(); handleSelect(id) }}
          className="w-full flex items-center gap-2 px-3 py-2 text-sm rounded-lg border border-zinc-300 dark:border-zinc-700 hover:bg-zinc-100 dark:hover:bg-zinc-800 transition-colors"
        >
          <Plus size={16} />
          New Chat
        </button>
      </div>

      {/* Conversation list */}
      <div className="flex-1 overflow-y-auto py-1">
        {sorted.length === 0 ? (
          <p className="px-3 py-8 text-xs text-zinc-400 text-center">
            No conversations yet
          </p>
        ) : (
          sorted.map(conv => {
            const isActive = conv.id === activeId
            const msgCount = messages.filter(m => m.conversationId === conv.id).length
            return (
              <div
                key={conv.id}
                onClick={() => handleSelect(conv.id)}
                onMouseEnter={() => setHovered(conv.id)}
                onMouseLeave={() => setHovered(null)}
                className={`
                  group mx-2 my-0.5 px-3 py-2 rounded-lg cursor-pointer text-sm transition-colors
                  ${isActive
                    ? "bg-blue-50 dark:bg-blue-900/20 text-blue-700 dark:text-blue-300"
                    : "hover:bg-zinc-100 dark:hover:bg-zinc-800"
                  }
                `}
              >
                <div className="flex items-start justify-between gap-1">
                  <span className="line-clamp-2 flex-1 text-xs font-medium leading-snug">
                    {conv.title}
                  </span>
                  {hovered === conv.id && (
                    <button
                      onClick={e => { e.stopPropagation(); deleteConversation(conv.id) }}
                      className="flex-shrink-0 p-0.5 rounded hover:bg-red-100 dark:hover:bg-red-900/30 text-zinc-400 hover:text-red-500"
                    >
                      <Trash2 size={12} />
                    </button>
                  )}
                </div>
                <div className="mt-0.5 flex items-center gap-1.5 text-[10px] text-zinc-400">
                  <span>{formatDate(conv.updatedAt)}</span>
                  {msgCount > 0 && <><span>·</span><span>{msgCount} msgs</span></>}
                </div>
              </div>
            )
          })
        )}
      </div>

      {/* Bottom actions */}
      <div className="border-t border-zinc-200 dark:border-zinc-800 p-3 flex items-center justify-between">
        <button
          onClick={toggleDark}
          className="p-1.5 rounded-lg hover:bg-zinc-100 dark:hover:bg-zinc-800 text-zinc-500"
          title={dark ? "Light mode" : "Dark mode"}
        >
          {dark ? <Sun size={16} /> : <Moon size={16} />}
        </button>
        <button
          onClick={() => setConfig({ configured: false })}
          className="p-1.5 rounded-lg hover:bg-zinc-100 dark:hover:bg-zinc-800 text-zinc-500"
          title="Settings"
        >
          <Settings size={16} />
        </button>
      </div>
    </div>
  )
}

function formatDate(ts: number): string {
  const d = new Date(ts)
  const now = new Date()
  if (d.toDateString() === now.toDateString()) {
    return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
  }
  return d.toLocaleDateString([], { month: "short", day: "numeric" })
}
