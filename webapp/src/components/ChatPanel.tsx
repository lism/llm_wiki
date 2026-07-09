import { useEffect, useRef } from "react"
import { useStore } from "../store"
import { chatStreaming, type Reference } from "../api"
import { ChatMessage } from "./ChatMessage"
import { ChatInput } from "./ChatInput"

export function ChatPanel() {
  const messages = useStore(s => s.messages)
  const activeId = useStore(s => s.activeConversationId)
  const isStreaming = useStore(s => s.isStreaming)
  const streamingContent = useStore(s => s.streamingContent)
  const addMessage = useStore(s => s.addMessage)
  const setStreaming = useStore(s => s.setStreaming)
  const appendStreamToken = useStore(s => s.appendStreamToken)
  const config = useStore(s => s.config)
  const bottomRef = useRef<HTMLDivElement>(null)
  const abortRef = useRef<AbortController | null>(null)

  const activeMessages = messages.filter(m => m.conversationId === activeId)

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" })
  }, [messages, streamingContent])

  function handleSend(text: string) {
    if (!text.trim() || !activeId || isStreaming) return

    addMessage({ role: "user", content: text })

    setStreaming(true)
    let answer = ""
    const refs: Reference[] = []

    const controller = chatStreaming(config.projectId, text, {
      onToken(token) {
        answer += token
        appendStreamToken(token)
      },
      onReferences(r) {
        refs.push(...r)
      },
      onDone() {
        setStreaming(false)
        addMessage({ role: "assistant", content: answer, references: refs })
      },
      onError(err) {
        setStreaming(false)
        if (answer) {
          // Partial stream — save what we have
          addMessage({ role: "assistant", content: answer + `\n\n[Stream interrupted: ${err}]`, references: refs })
        } else {
          addMessage({ role: "assistant", content: `Error: ${err}` })
        }
      },
    })
    abortRef.current = controller
  }

  function handleStop() {
    abortRef.current?.abort()
    setStreaming(false)
  }

  return (
    <div className="flex-1 flex flex-col min-h-0">
      {/* Messages */}
      <div className="flex-1 overflow-y-auto px-4 py-4 space-y-4">
        {activeMessages.length === 0 && !isStreaming && (
          <div className="flex items-center justify-center h-full text-sm text-zinc-400">
            Ask a question about your wiki
          </div>
        )}

        {activeMessages.map(msg => (
          <ChatMessage key={msg.id} message={msg} />
        ))}

        {/* Streaming message */}
        {isStreaming && streamingContent && (
          <ChatMessage
            message={{
              id: "streaming",
              role: "assistant",
              content: streamingContent,
              timestamp: Date.now(),
              conversationId: activeId || "",
            }}
            isStreaming
          />
        )}

        <div ref={bottomRef} />
      </div>

      {/* Input */}
      <ChatInput onSend={handleSend} onStop={handleStop} isStreaming={isStreaming} />
    </div>
  )
}
