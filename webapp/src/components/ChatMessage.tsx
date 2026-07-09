import { useState } from "react"
import ReactMarkdown from "react-markdown"
import remarkGfm from "remark-gfm"
import rehypeKatex from "rehype-katex"
import { User, Bot, ChevronDown, ChevronRight } from "lucide-react"
import type { Message } from "../store"

interface Props {
  message: Message
  isStreaming?: boolean
}

export function ChatMessage({ message, isStreaming }: Props) {
  const [refsOpen, setRefsOpen] = useState(false)
  const isUser = message.role === "user"
  const hasRefs = !isUser && message.references && message.references.length > 0

  return (
    <div className={`flex gap-3 ${isUser ? "justify-end" : ""}`}>
      {/* Avatar */}
      <div
        className={`
          flex-shrink-0 w-7 h-7 rounded-full flex items-center justify-center text-white text-xs
          ${isUser ? "order-last bg-blue-500" : "bg-emerald-600"}
        `}
      >
        {isUser ? <User size={14} /> : <Bot size={14} />}
      </div>

      {/* Content */}
      <div className={`min-w-0 max-w-[80%] ${isUser ? "text-right" : ""}`}>
        <div
          className={`
            rounded-2xl px-4 py-2.5 text-sm leading-relaxed
            ${isUser
              ? "bg-blue-600 text-white rounded-br-md"
              : "bg-zinc-100 dark:bg-zinc-800 rounded-bl-md"
            }
            ${isStreaming ? "streaming-cursor" : ""}
          `}
        >
          {isUser ? (
            <p className="whitespace-pre-wrap">{message.content}</p>
          ) : (
            <div className="prose prose-sm dark:prose-invert max-w-none">
              <ReactMarkdown
                remarkPlugins={[remarkGfm]}
                rehypePlugins={[rehypeKatex]}
                components={{
                  a({ href, children }) {
                    // Render [[wikilinks]] as bold text (link target doesn't exist in webapp)
                    if (href?.startsWith("[[")) {
                      const text = (href || "").replace(/^\[\[|\]\]$/g, "")
                      return <strong className="text-blue-600 dark:text-blue-400">{text}</strong>
                    }
                    return <a href={href} target="_blank" rel="noopener" className="text-blue-600 dark:text-blue-400 underline">{children}</a>
                  },
                }}
              >
                {message.content}
              </ReactMarkdown>
            </div>
          )}
        </div>

        {/* References toggle */}
        {hasRefs && (
          <div className="mt-1.5">
            <button
              onClick={() => setRefsOpen(!refsOpen)}
              className="inline-flex items-center gap-1 text-xs text-zinc-500 hover:text-zinc-700 dark:hover:text-zinc-300"
            >
              {refsOpen ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
              {message.references!.length} reference(s)
            </button>
            {refsOpen && (
              <div className="mt-1.5 space-y-1">
                {message.references!.map((ref, i) => (
                  <div
                    key={i}
                    className="text-xs bg-zinc-50 dark:bg-zinc-800/50 rounded-lg px-3 py-1.5 border border-zinc-200 dark:border-zinc-700"
                  >
                    <span className="font-medium">[{i + 1}] {ref.title}</span>
                    <span className="text-zinc-400 ml-2">{ref.path}</span>
                    {ref.snippet && (
                      <p className="text-zinc-500 mt-0.5 line-clamp-2">{ref.snippet}</p>
                    )}
                  </div>
                ))}
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  )
}
