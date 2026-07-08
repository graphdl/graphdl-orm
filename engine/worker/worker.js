// The AREST Worker: the whitepaper's REST surface (AREST.tex §Interop —
// "A POST /orders request creates an Order and returns its status with
// the actions available from it") over the wasm core's store_call.
// ONE verb table, THREE bindings now: MCP host, wasm arest_call, and
// these routes. The store loads once per isolate from the bound KV
// (or inline env var) sidecar payload.
//
// Routes (nouns sqlname-mangled; pluralization is a PLURAL map entry
// away if the model wants /orders over /order):
//   GET  /{noun}/{id}          -> get   (the 3NF view + hypermedia)
//   GET  /{noun}/{id}/actions  -> actions (status + transitions)
//   GET  /{noun}/{id}/repr     -> synthesize (the verbalized facts)
//   GET  /schema/{noun}        -> schema
//   POST /{noun}/{id}/{event}  -> apply of the transition's trigger
//                                 fact (WRITE PATH: step 5 — the
//                                 Worker is read-only until the write
//                                 story lands; 501 meanwhile)
import init, { arest_load, arest_call } from "./arest_core.js";
import wasmModule from "./arest_core_bg.wasm";
import SIDECAR from "./sidecar.js";

const PLURAL = {}; // e.g. { order: "orders" } — inverse applied on route parse

let ready = null;
async function ensure(env) {
  if (!ready) {
    ready = (async () => {
      await init(wasmModule);
      const sidecar =
        env?.SIDECAR_JSON ?? (env?.STORE ? await env.STORE.get("sidecar") : null) ?? SIDECAR;
      arest_load(sidecar);
    })();
  }
  return ready;
}

function sqlname(s) {
  const t = s.replace(/[^0-9A-Za-z]+/g, "_").replace(/^_+|_+$/g, "").toLowerCase();
  return t || "t";
}

export default {
  async fetch(request, env) {
    await ensure(env);
    const url = new URL(request.url);
    const seg = url.pathname.split("/").filter(Boolean);
    const json = (body, status = 200) =>
      new Response(body, { status, headers: { "content-type": "application/json" } });

    // the noun map: sqlname(noun) -> model noun, built per request off
    // the schema answer (cheap: the store is resident)
    const nounOf = (slug) => {
      const want = Object.entries(PLURAL).find(([, p]) => p === slug)?.[0] ?? slug;
      const schema = JSON.parse(arest_call("schema", "{}"));
      const nouns = schema.nouns ?? schema.object_types ?? [];
      for (const n of nouns) {
        const name = typeof n === "string" ? n : n.name;
        if (sqlname(name) === want) return name;
      }
      return null;
    };

    if (request.method === "GET" && seg[0] === "schema" && seg[1]) {
      const noun = nounOf(seg[1]);
      if (!noun) return json('{"error":"unknown noun"}', 404);
      return json(arest_call("schema", JSON.stringify({ noun })));
    }
    if (seg.length >= 2) {
      const noun = nounOf(seg[0]);
      if (!noun) return json('{"error":"unknown noun"}', 404);
      const id = decodeURIComponent(seg[1]);
      if (request.method === "GET") {
        if (seg[2] === "actions")
          return json(arest_call("actions", JSON.stringify({ noun, id })));
        if (seg[2] === "repr")
          return json(arest_call("synthesize", JSON.stringify({ id })));
        return json(arest_call("get", JSON.stringify({ noun, id })));
      }
      if (request.method === "POST" && seg[2])
        return json('{"error":"the write path is step 5"}', 501);
    }
    return json('{"error":"routes: GET /{noun}/{id}[/actions|/repr], GET /schema/{noun}"}', 404);
  },
};
