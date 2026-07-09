import { useRef, useCallback } from "react"
import { Send, Square } from "lucide-react"

interface Props {
  onSend: (text: string) => void
  onStop: () => void
  isStreaming: boolean
}

export function ChatInput({ onSend, onStop, isStreaming }: Props) {
  const ref = useRef<HTMLTextAreaElement>(null)

  const handleSend = useCallback(() => {
    const text = ref.current?.value.trim()
    if (!text || isStreaming) return
    onSend(text)
    ref.current!.value = ""
    ref.current!.style.height = "auto"
  }, [onSend, isStreaming])

  function handleKeyDown(e: React.KeyboardEvent) {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault()
      handleSend()
    }
  }

  function handleInput() {
    const el = ref.current
    if (!el) return
    el.style.height = "auto"
    el.style.height = Math.min(el.scrollHeight, 200) + "px"
  }

  return (
    <div className="border-t border-zinc-200 dark:border-zinc-800 p-4">
      <div className="flex items-end gap-2 max-w-3xl mx-auto">
        <textarea
          ref={ref}
          onKeyDown={handleKeyDown}
          onInput={handleInput}
          placeholder="Ask about your wiki..."
          rows={1}
          className="flex-1 resize-none rounded-xl border border-zinc-300 dark:border-zinc-600 bg-white dark:bg-zinc-800 px-4 py-2.5 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500 placeholder:text-zinc-400"
        />
        {isStreaming ? (
          <button
            onClick={onStop}
            className="flex-shrink-0 p-2.5 rounded-xl bg-red-500 hover:bg-red-600 text-white transition-colors"
          >
            <Square size={16} fill="currentColor" />
          </button>
        ) : (
          <button
            onClick={handleSend}
            className="flex-shrink-0 p-2.5 rounded-xl bg-blue-600 hover:bg-blue-700 text-white transition-colors disabled:opacity-50"
          >
            <Send size={16} />
          </button>
        )}
      </div>
    </div>
  )
}
