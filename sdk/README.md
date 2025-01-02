# GraphDL SDK

The GraphDL SDK is a TypeScript library that provides a simple and convenient way to interact with the GraphDL API. It is built on top of the `payload-rest-client` library and uses environment variables for configuration.

## Installation

```bash
npm install --save graphdl-sdk
```

or yarn:

```bash
yarn add graphdl-sdk
```

## Usage

First, import the SDK into your project:

```typescript
import { SDK } from 'graphdl-sdk'
```

Then, initialize the SDK with your API key:

```typescript
const sdk = SDK('your-api-key')
```

If you don't provide an API key, the SDK will try to use the `GRAPHDL_API_KEY` environment variable.

## API

The SDK provides access to various collections in the GraphDL API:

- `sdk.things`
- `sdk.nouns`
- `sdk.resources`
- `sdk.verbs`
- `sdk.actions`
- `sdk.constraints`
- `sdk.roles`
- `sdk.graphSchemas`
- `sdk.graphs`
- `sdk.eventTypes`
- `sdk.events`
- `sdk.data`
- `sdk.streams`
- `sdk.states`
- `sdk.stateMachineDefinitions`
- `sdk.stateMachines`
- `sdk.transitions`
- `sdk.guardExpressionRunTypes`
- `sdk.guardExpressionRuns`

Each of these collections corresponds to a specific part of the GraphDL API and can be used to perform CRUD operations.

## Environment Variables

The SDK uses the following environment variables:

- `GRAPHDL_API_KEY`: Your GraphDL API key.

## Contributing

Contributions are welcome! Please read our contributing guidelines before getting started.

## License

This project is licensed under the terms of the MIT license.
