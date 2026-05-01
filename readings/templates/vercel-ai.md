# Vercel AI SDK

Reading for the `ai` npm package (Vercel AI SDK core) and its provider
modules. Verbs declared here become callable through DEFS once the JS
import mechanism described in `core/imports.md` resolves them.

The core package exports the LLM call surface (`generateText`,
`streamText`, `generateObject`, `streamObject`, `embed`, `embedMany`,
`tool`) plus stream-to-Response helpers used on the server side. Provider
modules export Model factories that the core functions consume.

## Instance Facts

### Packages

JS Package 'ai' has Version '^4.0.0'.
JS Package 'ai' has Description 'Vercel AI SDK core: generateText, streamText, generateObject, streamObject, tool, embed, embedMany. Provider-agnostic surface that consumes Model objects from any @ai-sdk/* provider package.'.
JS Package 'ai' has Package Manager 'npm'.

JS Package '@ai-sdk/openai' has Version '^1.0.0'.
JS Package '@ai-sdk/openai' has Description 'OpenAI provider for Vercel AI SDK. Exports openai() Model factory and openai.responses() / openai.chat() variants.'.
JS Package '@ai-sdk/openai' has Package Manager 'npm'.

JS Package '@ai-sdk/anthropic' has Version '^1.0.0'.
JS Package '@ai-sdk/anthropic' has Description 'Anthropic provider for Vercel AI SDK. Exports anthropic() Model factory.'.
JS Package '@ai-sdk/anthropic' has Package Manager 'npm'.

JS Package '@ai-sdk/google' has Version '^1.0.0'.
JS Package '@ai-sdk/google' has Description 'Google Generative AI provider for Vercel AI SDK. Exports google() Model factory.'.
JS Package '@ai-sdk/google' has Package Manager 'npm'.

JS Package '@ai-sdk/xai' has Version '^1.0.0'.
JS Package '@ai-sdk/xai' has Description 'xAI provider for Vercel AI SDK. Exports xai() Model factory for Grok models.'.
JS Package '@ai-sdk/xai' has Package Manager 'npm'.

### Core call surface

Verb 'generateText' is exported from JS Package 'ai'.
Verb 'generateText' has Module Path 'ai'.
Verb 'generateText' has Symbol Name 'generateText'.
Verb 'generateText' has Description 'Generate text and tool calls non-streaming. Takes {model, messages | prompt, tools, system, maxTokens, temperature, ...}; returns {text, toolCalls, toolResults, finishReason, usage, ...}.'.

Verb 'streamText' is exported from JS Package 'ai'.
Verb 'streamText' has Module Path 'ai'.
Verb 'streamText' has Symbol Name 'streamText'.
Verb 'streamText' has Description 'Generate text and tool calls streaming. Takes {model, messages | prompt, tools, ...}; returns StreamTextResult with textStream / fullStream async iterators plus toDataStreamResponse() / toAIStream() converters.'.

Verb 'generateObject' is exported from JS Package 'ai'.
Verb 'generateObject' has Module Path 'ai'.
Verb 'generateObject' has Symbol Name 'generateObject'.
Verb 'generateObject' has Description 'Generate a typed object matching a schema. Takes {model, schema, messages | prompt, mode, ...}; returns {object, finishReason, usage, ...}. Schema is a Zod / valibot / JSON Schema definition.'.

Verb 'streamObject' is exported from JS Package 'ai'.
Verb 'streamObject' has Module Path 'ai'.
Verb 'streamObject' has Symbol Name 'streamObject'.
Verb 'streamObject' has Description 'Stream a typed object matching a schema. Takes {model, schema, messages | prompt, ...}; returns StreamObjectResult with partialObjectStream async iterator.'.

Verb 'embed' is exported from JS Package 'ai'.
Verb 'embed' has Module Path 'ai'.
Verb 'embed' has Symbol Name 'embed'.
Verb 'embed' has Description 'Generate an embedding vector for a single value. Takes {model, value}; returns {embedding, usage, ...}.'.

Verb 'embedMany' is exported from JS Package 'ai'.
Verb 'embedMany' has Module Path 'ai'.
Verb 'embedMany' has Symbol Name 'embedMany'.
Verb 'embedMany' has Description 'Generate embedding vectors for many values in one call. Takes {model, values}; returns {embeddings, usage, ...}.'.

Verb 'tool' is exported from JS Package 'ai'.
Verb 'tool' has Module Path 'ai'.
Verb 'tool' has Symbol Name 'tool'.
Verb 'tool' has Description 'Define a tool with description, parameters schema, and execute function. Returned ToolDefinition is passed in the tools map of generateText / streamText. The model calls the tool via Tool Call (templates/agent-chat.md).'.

Verb 'jsonSchema' is exported from JS Package 'ai'.
Verb 'jsonSchema' has Module Path 'ai'.
Verb 'jsonSchema' has Symbol Name 'jsonSchema'.
Verb 'jsonSchema' has Description 'Wrap a raw JSON Schema for use with generateObject / streamObject when not using Zod.'.

### Provider model factories

Verb 'openai' is exported from JS Package '@ai-sdk/openai'.
Verb 'openai' has Module Path '@ai-sdk/openai'.
Verb 'openai' has Symbol Name 'openai'.
Verb 'openai' has Description 'Construct an OpenAI Model. openai("gpt-4o") returns a Model. openai.responses("gpt-4o") opts into the Responses API.'.

Verb 'anthropic' is exported from JS Package '@ai-sdk/anthropic'.
Verb 'anthropic' has Module Path '@ai-sdk/anthropic'.
Verb 'anthropic' has Symbol Name 'anthropic'.
Verb 'anthropic' has Description 'Construct an Anthropic Model. anthropic("claude-opus-4-7") returns a Model.'.

Verb 'google' is exported from JS Package '@ai-sdk/google'.
Verb 'google' has Module Path '@ai-sdk/google'.
Verb 'google' has Symbol Name 'google'.
Verb 'google' has Description 'Construct a Google Generative AI Model. google("gemini-1.5-pro") returns a Model.'.

Verb 'xai' is exported from JS Package '@ai-sdk/xai'.
Verb 'xai' has Module Path '@ai-sdk/xai'.
Verb 'xai' has Symbol Name 'xai'.
Verb 'xai' has Description 'Construct an xAI Model. xai("grok-2") returns a Model.'.

### Stream-to-Response helpers

Verb 'streamToResponse' is exported from JS Package 'ai'.
Verb 'streamToResponse' has Module Path 'ai'.
Verb 'streamToResponse' has Symbol Name 'streamToResponse'.
Verb 'streamToResponse' has Description 'Convert a StreamTextResult into a Response that the client useChat hook can consume. Wires the SSE protocol the hook expects.'.

Domain 'vercel-ai' has Access 'public'.
Domain 'vercel-ai' has Description 'Vercel AI SDK (npm: ai). Core primitives for building LLM applications: generateText, streamText, generateObject, streamObject, embed, embedMany, tool. Provider modules (@ai-sdk/openai, @ai-sdk/anthropic, @ai-sdk/google, @ai-sdk/xai) export model factories. Verbs declared here become callable to AREST domains via DEFS once core/imports.md is wired into the runtime; pair with templates/vercel-chat.md for the React UI surface.'.
