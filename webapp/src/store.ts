import { create } from "zustand"
import type { Reference } from "./api"

// ── types ─────────────────────────────────────────────────────────

export interface Message {
  id: string
  conversationId: string
  role: "user" | "assistant"
  content: string
  timestamp: number
  references?: Reference[]
}

export interface Conversation {
  id: string
  title: string
  createdAt: number
  updatedAt: number
}

interface AppConfig {
  baseUrl: string
  token: string
  projectId: string
  configured: boolean
}

interface ChatState {
  // Config
  config: AppConfig
  setConfig: (c: Partial<AppConfig>) => void

  // Conversations
  conversations: Conversation[]
  activeConversationId: string | null
  createConversation: () => string
  deleteConversation: (id: string) => void
  setActiveConversation: (id: string) => void

  // Messages
  messages: Message[]
  addMessage: (msg: Omit<Message, "id" | "timestamp" | "conversationId">) => void
  clearMessages: () => void

  // Streaming
  isStreaming: boolean
  streamingContent: string
  setStreaming: (v: boolean) => void
  appendStreamToken: (t: string) => void

  // Theme
  dark: boolean
  toggleDark: () => void
}

// ── helpers ───────────────────────────────────────────────────────

let msgCounter = 0
function nextId() { msgCounter++; return String(msgCounter) }
function convId() { return `conv_${Date.now()}_${Math.random().toString(36).slice(2, 8)}` }

const CONFIG_KEY = "llm-wiki-webapp-config"
const CONVS_KEY = "llm-wiki-webapp-conversations"
const MSGS_KEY = "llm-wiki-webapp-messages"

function loadConfig(): AppConfig {
  try {
    const raw = localStorage.getItem(CONFIG_KEY)
    if (raw) return JSON.parse(raw)
  } catch { /* ignore */ }
  return { baseUrl: "http://127.0.0.1:19828", token: "", projectId: "", configured: false }
}

function saveConfig(c: AppConfig) {
  localStorage.setItem(CONFIG_KEY, JSON.stringify(c))
}

function loadConversations(): Conversation[] {
  try {
    const raw = localStorage.getItem(CONVS_KEY)
    if (raw) return JSON.parse(raw)
  } catch { /* ignore */ }
  return []
}

function saveConversations(convs: Conversation[]) {
  localStorage.setItem(CONVS_KEY, JSON.stringify(convs))
}

function loadMessages(): Message[] {
  try {
    const raw = localStorage.getItem(MSGS_KEY)
    if (raw) return JSON.parse(raw)
  } catch { /* ignore */ }
  return []
}

function saveMessages(msgs: Message[]) {
  // Keep only last 500 messages to not blow up localStorage
  const trimmed = msgs.slice(-500)
  localStorage.setItem(MSGS_KEY, JSON.stringify(trimmed))
}

// ── store ─────────────────────────────────────────────────────────

export const useStore = create<ChatState>((set, get) => ({
  config: loadConfig(),
  setConfig: (partial) => {
    const next = { ...get().config, ...partial }
    saveConfig(next)
    set({ config: next })
  },

  conversations: loadConversations(),
  activeConversationId: null,
  createConversation: () => {
    const id = convId()
    const conv: Conversation = {
      id,
      title: "New Chat",
      createdAt: Date.now(),
      updatedAt: Date.now(),
    }
    const convs = [...get().conversations, conv]
    saveConversations(convs)
    set({ conversations: convs, activeConversationId: id })
    return id
  },
  deleteConversation: (id) => {
    const convs = get().conversations.filter(c => c.id !== id)
    saveConversations(convs)
    const msgs = get().messages.filter(m => m.conversationId !== id)
    saveMessages(msgs)
    const nextActive = get().activeConversationId === id
      ? (convs[convs.length - 1]?.id ?? null)
      : get().activeConversationId
    set({ conversations: convs, messages: msgs, activeConversationId: nextActive })
  },
  setActiveConversation: (id) => set({ activeConversationId: id }),

  messages: loadMessages(),
  addMessage: (msg) => {
    if (!get().activeConversationId) return
    const message: Message = {
      ...msg,
      id: nextId(),
      conversationId: get().activeConversationId!,
      timestamp: Date.now(),
    }
    const msgs = [...get().messages, message]
    saveMessages(msgs)
    // Update conversation timestamp + title (first user message)
    const convs = get().conversations.map(c => {
      if (c.id !== get().activeConversationId) return c
      const updates: Partial<Conversation> = { updatedAt: Date.now() }
      if (msg.role === "user" && c.title === "New Chat") {
        updates.title = msg.content.slice(0, 40) || "Chat"
      }
      return { ...c, ...updates }
    })
    saveConversations(convs)
    set({ messages: msgs, conversations: convs })
  },
  clearMessages: () => {
    saveMessages([])
    set({ messages: [] })
  },

  isStreaming: false,
  streamingContent: "",
  setStreaming: (v) => set({ isStreaming: v, streamingContent: v ? "" : get().streamingContent }),
  appendStreamToken: (t) => set(s => ({ streamingContent: s.streamingContent + t })),

  dark: (() => {
    const stored = localStorage.getItem("llm-wiki-webapp-dark")
    if (stored !== null) return stored === "true"
    return window.matchMedia("(prefers-color-scheme: dark)").matches
  })(),
  toggleDark: () => {
    const next = !get().dark
    localStorage.setItem("llm-wiki-webapp-dark", String(next))
    set({ dark: next })
  },
}))
