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
import init, { arest_load, arest_call, arest_apply } from "./arest_core.js";
import wasmModule from "./arest_core_bg.wasm";
import SIDECAR from "./sidecar.js";

const PLURAL = {}; // e.g. { order: "orders" } — inverse applied on route parse

// The event-log Durable Object: Def. iso's single-writer cell, worker
// storage edition — ONE instance per app, append = the commit point,
// the stream is the store of record. The isolate's wasm store is the
// snapshot (the bundled sidecar) plus this log's tail, replayed at
// boot exactly like the resident's stream-watermark discipline.
export class ArestLog {
  constructor(state) {
    this.state = state;
  }
  async fetch(request) {
    const url = new URL(request.url);
    if (request.method === "POST" && url.pathname === "/append") {
      const line = await request.text();
      const n = (await this.state.storage.get("n")) ?? 0;
      await this.state.storage.put(`e:${String(n).padStart(9, "0")}`, line);
      await this.state.storage.put("n", n + 1);
      return new Response(JSON.stringify({ appended: n + 1 }));
    }
    if (request.method === "GET" && url.pathname === "/tail") {
      const n = (await this.state.storage.get("n")) ?? 0;
      const out = [];
      const rows = await this.state.storage.list({ prefix: "e:" });
      for (const [, v] of rows) out.push(v);
      return new Response(JSON.stringify({ n, events: out }));
    }
    return new Response("{}", { status: 404 });
  }
}

let ready = null;
async function ensure(env) {
  if (!ready) {
    ready = (async () => {
      await init(wasmModule);
      const sidecar =
        env?.SIDECAR_JSON ?? (env?.STORE ? await env.STORE.get("sidecar") : null) ?? SIDECAR;
      arest_load(sidecar);
      // replay the log tail over the snapshot (a replayed duplicate
      // reads as refused — D' == D — and changes nothing)
      if (env?.LOG) {
        const id = env.LOG.idFromName("log");
        const stub = env.LOG.get(id);
        const r = await stub.fetch("https://log/tail");
        const { events } = await r.json();
        for (const line of events) {
          try {
            const e = JSON.parse(line);
            arest_apply(JSON.stringify({ fact_type: e.ft, fact: e.fact }));
          } catch {}
        }
      }
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
    if (request.method === "POST" && seg.length === 1) {
      // POST /{noun}: the whitepaper's create ("A POST /orders request
      // creates an Order...") — the body names the fact: {fact_type,
      // fact}. Commit appends to the Durable Object stream.
      const noun = nounOf(seg[0]);
      if (!noun) return json('{"error":"unknown noun"}', 404);
      let body = null;
      try { body = await request.json(); } catch {}
      if (!body?.fact_type || !Array.isArray(body?.fact))
        return json('{"error":"body needs fact_type and fact"}', 400);
      const r = JSON.parse(arest_apply(JSON.stringify(
        { fact_type: body.fact_type, fact: body.fact })));
      if (r.receipt?.committed && r.event && env?.LOG) {
        const idd = env.LOG.idFromName("log");
        await env.LOG.get(idd).fetch("https://log/append", {
          method: "POST", body: JSON.stringify(r.event) });
      }
      return json(JSON.stringify(r), r.receipt?.committed ? 201 : 422);
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
      if (request.method === "POST" && seg[2]) {
        // POST /{noun}/{id}/{event}: the transition's trigger fact —
        // the whitepaper's "following the action" (the actions answer
        // names the event's fact type). Default fact = ⟨id⟩ (a unary
        // trigger); a body {"fact": [...]} overrides.
        const actions = JSON.parse(
          arest_call("actions", JSON.stringify({ noun, id })));
        const hit = (actions.actions || []).find(
          (a) => a.event === seg[2] || sqlname(a.event) === seg[2]);
        if (!hit) return json('{"error":"no such transition from this status"}', 409);
        let fact = [id];
        try {
          const body = await request.json();
          if (Array.isArray(body?.fact)) fact = body.fact;
        } catch {}
        const r = JSON.parse(
          arest_apply(JSON.stringify({ fact_type: hit.event, fact })));
        if (r.receipt?.committed && r.event && env?.LOG) {
          const idd = env.LOG.idFromName("log");
          await env.LOG.get(idd).fetch("https://log/append", {
            method: "POST",
            body: JSON.stringify(r.event),
          });
        }
        return json(JSON.stringify(r), r.receipt?.committed ? 200 : 422);
      }
    }
    return json('{"error":"routes: GET /{noun}/{id}[/actions|/repr], GET /schema/{noun}"}', 404);
  },
};
