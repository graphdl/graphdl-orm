# Vercel AI SDK Chat UI

## Instance Facts

### Packages

JS Package '@ai-sdk/react' has Version '^1.0.0'.
JS Package '@ai-sdk/react' has Description 'Vercel AI SDK React UI bindings. Hooks (useChat, useCompletion, useObject) for managed chat, completion, and structured-object UI state.'.
JS Package '@ai-sdk/react' has Package Manager 'npm'.

JS Package '@ai-sdk/vue' has Version '^1.0.0'.
JS Package '@ai-sdk/vue' has Description 'Vercel AI SDK Vue UI bindings. useChat / useCompletion equivalents for Vue 3.'.
JS Package '@ai-sdk/vue' has Package Manager 'npm'.

JS Package '@ai-sdk/svelte' has Version '^1.0.0'.
JS Package '@ai-sdk/svelte' has Description 'Vercel AI SDK Svelte UI bindings. useChat / useCompletion equivalents for Svelte 5.'.
JS Package '@ai-sdk/svelte' has Package Manager 'npm'.

JS Package 'ai-elements' has Version '^0.1.0'.
JS Package 'ai-elements' has Description 'Pre-built React chat UI primitives layered on @ai-sdk/react: Conversation, Message, PromptInput, Response, Reasoning. Composable shadcn-style components.'.
JS Package 'ai-elements' has Package Manager 'npm'.

### React hooks

Verb 'useChat' is exported from JS Package '@ai-sdk/react'.
Verb 'useChat' has Module Path '@ai-sdk/react'.
Verb 'useChat' has Symbol Name 'useChat'.
Verb 'useChat' has Description 'React hook for managed multi-turn chat state. Takes {api, id, initialMessages, body, headers, onFinish, onError, ...}; returns {messages, input, handleInputChange, handleSubmit, append, reload, stop, isLoading, error, setMessages, setInput, data}. Consumes the AI SDK Data Stream protocol over SSE.'.

Verb 'useCompletion' is exported from JS Package '@ai-sdk/react'.
Verb 'useCompletion' has Module Path '@ai-sdk/react'.
Verb 'useCompletion' has Symbol Name 'useCompletion'.
Verb 'useCompletion' has Description 'React hook for single-turn text completion. Takes {api, id, initialInput, body, headers, onFinish, onError, ...}; returns {completion, input, handleInputChange, handleSubmit, complete, stop, isLoading, error, setCompletion, setInput}.'.

Verb 'useObject' is exported from JS Package '@ai-sdk/react'.
Verb 'useObject' has Module Path '@ai-sdk/react'.
Verb 'useObject' has Symbol Name 'experimental_useObject'.
Verb 'useObject' has Description 'React hook for streaming a typed object matching a schema. Takes {api, schema, id, headers, ...}; returns {object, submit, isLoading, error, stop}. Pairs with server-side streamObject.'.

Verb 'useAssistant' is exported from JS Package '@ai-sdk/react'.
Verb 'useAssistant' has Module Path '@ai-sdk/react'.
Verb 'useAssistant' has Symbol Name 'useAssistant'.
Verb 'useAssistant' has Description 'React hook for OpenAI Assistants API integration. Takes {api, threadId, ...}; returns {messages, input, status, threadId, append, submitMessage, ...}.'.

### AI Elements components

Verb 'Conversation' is exported from JS Package 'ai-elements'.
Verb 'Conversation' has Module Path 'ai-elements/conversation'.
Verb 'Conversation' has Symbol Name 'Conversation'.
Verb 'Conversation' has Description 'Scrollable container for chat messages. Wraps the messages array from useChat and provides auto-scroll, sticky bottom, and overflow handling.'.

Verb 'Message' is exported from JS Package 'ai-elements'.
Verb 'Message' has Module Path 'ai-elements/message'.
Verb 'Message' has Symbol Name 'Message'.
Verb 'Message' has Description 'Per-message bubble rendering a single chat turn. Takes {from: "user" | "assistant"} and content; styles user vs assistant variants.'.

Verb 'PromptInput' is exported from JS Package 'ai-elements'.
Verb 'PromptInput' has Module Path 'ai-elements/prompt-input'.
Verb 'PromptInput' has Symbol Name 'PromptInput'.
Verb 'PromptInput' has Description 'Composable input form for chat. Wraps useChat.handleSubmit + handleInputChange with submit-on-Enter behavior, attachment slots, and stop button.'.

Verb 'Response' is exported from JS Package 'ai-elements'.
Verb 'Response' has Module Path 'ai-elements/response'.
Verb 'Response' has Symbol Name 'Response'.
Verb 'Response' has Description 'Markdown renderer for streaming assistant text. Handles partial streaming gracefully and applies syntax highlighting to code blocks.'.

Verb 'Reasoning' is exported from JS Package 'ai-elements'.
Verb 'Reasoning' has Module Path 'ai-elements/reasoning'.
Verb 'Reasoning' has Symbol Name 'Reasoning'.
Verb 'Reasoning' has Description 'Collapsible disclosure for thinking-mode model output. Renders the reasoning channel from streamText results without dominating the visible chat.'.

### Server-side handlers (paired with hooks)

Verb 'toDataStreamResponse' is exported from JS Package 'ai'.
Verb 'toDataStreamResponse' has Module Path 'ai'.
Verb 'toDataStreamResponse' has Symbol Name 'toDataStreamResponse'.
Verb 'toDataStreamResponse' has Description 'Method on StreamTextResult that emits the AI SDK Data Stream protocol. The Response returned by this method is what useChat / useObject expect from the {api} URL.'.

Verb 'toAIStreamResponse' is exported from JS Package 'ai'.
Verb 'toAIStreamResponse' has Module Path 'ai'.
Verb 'toAIStreamResponse' has Symbol Name 'toAIStreamResponse'.
Verb 'toAIStreamResponse' has Description 'Legacy converter for older streaming clients. Prefer toDataStreamResponse for new code.'.

Domain 'vercel-chat' has Access 'public'.
Domain 'vercel-chat' has Description 'Vercel AI SDK React UI surface (@ai-sdk/react) plus AI Elements components.'.
