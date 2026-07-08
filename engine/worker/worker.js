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

const INDEX_HTML = `<!doctype html>
<html>
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>arest</title>
<style>
  :root { color-scheme: light dark; font-family: system-ui, sans-serif; }
  body { margin: 2rem auto; max-width: 46rem; padding: 0 1rem; line-height: 1.5; }
  h1 { font-size: 1.2rem; letter-spacing: .06em; }
  input, button, select { font: inherit; padding: .4rem .6rem; }
  dl { display: grid; grid-template-columns: max-content 1fr; gap: .25rem .9rem; }
  dt { opacity: .65; }
  .actions button { margin-right: .5rem; }
  .bar { display: flex; gap: .5rem; margin: 1rem 0; flex-wrap: wrap; }
  .receipt { font-size: .85rem; opacity: .75; margin-top: 1rem; white-space: pre-wrap; }
</style>
</head>
<body>
<div id="root"></div>
<script type="module">
import React from "https://esm.sh/react@19";
import { createRoot } from "https://esm.sh/react-dom@19/client";
const h = React.createElement;

// THE BINDING CONTRACT: a component receives the representation and
// the apply endpoint — nothing else. Facts render; events apply; the
// store is the state (every commit refetches the representation).
function Field({ name, value }) {
  return h(React.Fragment, null,
    h("dt", null, name),
    h("dd", null, value === null ? h("i", null, "—") : String(value)));
}
function Detail({ view }) {
  return h("dl", null,
    Object.entries(view.fields || {}).map(([k, v]) =>
      h(Field, { key: k, name: k, value: v })));
}
function Actions({ actions, onFollow }) {
  if (!actions?.length) return null;
  return h("p", { className: "actions" },
    actions.map(a => h("button", {
      key: a.event,
      onClick: () => onFollow(a),   // the event IS a fact: apply it
    }, a.event + " → " + a.to)));
}

function App() {
  const [noun, setNoun] = React.useState("feature_request");
  const [id, setId] = React.useState("fr-live-1");
  const [view, setView] = React.useState(null);
  const [actions, setActions] = React.useState([]);
  const [receipt, setReceipt] = React.useState("");
  const refetch = React.useCallback(async () => {
    const v = await (await fetch("/" + noun + "/" + id)).json();
    setView(v);
    const a = await (await fetch("/" + noun + "/" + id + "/actions")).json();
    setActions(a.actions || []);
  }, [noun, id]);
  const follow = async (a) => {
    const r = await fetch("/" + noun + "/" + id + "/" + a.event, { method: "POST" });
    setReceipt(JSON.stringify(await r.json(), null, 1));
    await refetch();               // the store is the state
  };
  return h("div", null,
    h("h1", null, "AREST — a fact renders itself"),
    h("div", { className: "bar" },
      h("input", { value: noun, onChange: e => setNoun(e.target.value) }),
      h("input", { value: id, onChange: e => setId(e.target.value) }),
      h("button", { onClick: refetch }, "fetch")),
    view && h(Detail, { view }),
    h(Actions, { actions, onFollow: follow }),
    receipt && h("div", { className: "receipt" }, receipt));
}
createRoot(document.getElementById("root")).render(h(App));
</script>
</body>
</html>`;

export default {
  async fetch(request, env) {
    const url0 = new URL(request.url);
    if (request.method === "GET" && url0.pathname === "/") {
      return new Response(INDEX_HTML, {
        headers: { "content-type": "text/html; charset=utf-8" } });
    }
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
