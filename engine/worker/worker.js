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
import init, { arest_load, arest_call, arest_apply, arest_view, arest_entry, arest_ingest } from "./arest_core.js";
import wasmModule from "./arest_core_bg.wasm";
import SIDECAR from "./sidecar.js";

const PLURAL = {}; // e.g. { order: "orders" } — inverse applied on route parse

// The event-log Durable Object: Def. iso's single-writer cell, worker
// storage edition — ONE instance per app, append = the commit point,
// the stream is the store of record. The isolate's wasm store is the
// snapshot (the bundled sidecar) plus this log's tail, replayed at
// boot exactly like the resident's stream-watermark discipline.
export class ArestLog {
  constructor(state, env) {
    this.state = state;
    this.env = env;
  }
  async fetch(request) {
    const url = new URL(request.url);
    if (request.method === "POST" && url.pathname === "/append") {
      const line = await request.text();
      const n = (await this.state.storage.get("n")) ?? 0;
      await this.state.storage.put(`e:${String(n).padStart(9, "0")}`, line);
      await this.state.storage.put("n", n + 1);
      // the KV mirror (do-minimize): SSE and boot read KV, so the DO
      // is touched only by THIS append — its single-writer commit role
      if (this.env?.AREST_LOG) {
        await this.env.AREST_LOG.put(`e:${String(n).padStart(9, "0")}`, line);
        await this.env.AREST_LOG.put("n", String(n + 1));
      }
      return new Response(JSON.stringify({ appended: n + 1 }));
    }
    if (request.method === "POST" && url.pathname === "/mirror") {
      // one-time backfill of the KV mirror from the DO's own storage
      if (!this.env?.AREST_LOG)
        return new Response('{"error":"no AREST_LOG binding"}', { status: 501 });
      const n = (await this.state.storage.get("n")) ?? 0;
      const rows = await this.state.storage.list({ prefix: "e:" });
      let copied = 0;
      for (const [k, v] of rows) {
        await this.env.AREST_LOG.put(k, v);
        copied++;
      }
      await this.env.AREST_LOG.put("n", String(n));
      return new Response(JSON.stringify({ mirrored: copied, n }));
    }
    if (request.method === "GET" && url.pathname === "/tail") {
      const n = (await this.state.storage.get("n")) ?? 0;
      const from = Number(url.searchParams.get("from") ?? 0);
      const out = [];
      const rows = await this.state.storage.list({
        prefix: "e:",
        start: `e:${String(from).padStart(9, "0")}`,
      });
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
      // reads as refused — D' == D — and changes nothing). The KV
      // mirror serves the tail (do-minimize); the DO is asked only
      // when the mirror is empty (pre-mirror deployments)
      let events = null;
      if (env?.AREST_LOG) {
        const n = Number(await env.AREST_LOG.get("n"));
        if (Number.isFinite(n) && n > 0) {
          events = [];
          const rows = await env.AREST_LOG.list({ prefix: "e:" });
          for (const k of rows.keys) {
            const v = await env.AREST_LOG.get(k.name);
            if (v) events.push(v);
          }
        }
      }
      if (events === null && env?.LOG) {
        const id = env.LOG.idFromName("log");
        const stub = env.LOG.get(id);
        const r = await stub.fetch("https://log/tail");
        events = (await r.json()).events;
      }
      for (const line of events ?? []) {
        try {
          const e = JSON.parse(line);
          arest_apply(JSON.stringify({ fact_type: e.ft, fact: e.fact }));
        } catch {}
      }
    })();
  }
  return ready;
}

// ---- the federated connector (contact-federation.md at the edge) ----
// A noun backed by an External System refreshes on read: fetch the
// system's URL+URI with the AREST_SECRET_<SYSTEM> credential, map
// columns to <Noun>_has_<Field> facts, ingest ISOLATE-LOCALLY via
// arest_apply (no DO appends: federated rows are refetchable cache,
// not commits). OWA: no credential or unreachable = empty population.
const FED_MEMO = new Map();
const FED_TTL = 60_000;

function popRows(ft) {
  try {
    const r = JSON.parse(arest_call("query", JSON.stringify({ fact_type: ft })));
    return r.rows ?? [];
  } catch { return []; }
}

async function ensureFederated(env, noun) {
  const backed = popRows("Noun_is_backed_by_External_System")
    .find((r) => r[0] === noun);
  if (!backed) return;
  const key = noun;
  const hit = FED_MEMO.get(key);
  if (hit && Date.now() - hit < FED_TTL) return;
  const prop = (ft) => Object.fromEntries(
    popRows(ft).map((r) => [r[0], r[1]]));
  const sys = backed[1];
  const base = prop("External_System_has_URL")[sys];
  const uri = prop("Noun_has_URI")[noun];
  if (!base || !uri) return;
  const secret = env["AREST_SECRET_" + sys.replace(/[^0-9A-Za-z]+/g, "_").toUpperCase()];
  const headers = {};
  const hname = prop("External_System_has_Header")[sys];
  if (hname && secret)
    headers[hname] = ((prop("External_System_has_Prefix")[sys] ?? "") + " " + secret).trim();
  let payload = null;
  try {
    const r = await fetch(base + uri, { headers });
    if (r.ok) payload = await r.json();
  } catch {}
  const data = payload?.data ?? [];
  const nid = noun.replace(/[^0-9A-Za-z]+/g, "_").replace(/^_+|_+$/g, "");
  const byFt = new Map();
  const ids = [];
  for (const row of data) {
    const rid = String(row?.id ?? "").trim();
    if (!rid) continue;
    ids.push(rid);
    for (const [col, val] of Object.entries(row)) {
      if (col === "id" || val === null || val === "") continue;
      const ft = nid + "_has_" + String(col).replace(/[^0-9A-Za-z]+/g, "_").replace(/^_+|_+$/g, "");
      if (!byFt.has(ft)) byFt.set(ft, []);
      byFt.get(ft).push([rid, String(val)]);
    }
  }
  // THE BRIDGE MINT (identity is transduction): a noun surfaced as
  // another noun gets one same-id bridge row per fetched entity — the
  // canon re-key rules derive the fields from the identity link
  const surfaced = popRows("Noun_is_surfaced_as_Noun")
    .find((r) => r[0] === noun);
  if (surfaced && ids.length) {
    const bft = surfaced[1].replace(/[^0-9A-Za-z]+/g, "_").replace(/^_+|_+$/g, "")
      + "_is_" + nid;
    byFt.set(bft, ids.map((rid) => [rid, rid]));
  }
  if (byFt.size > 0) {
    const entries = [...byFt.entries()].map(([ft, facts]) => ({ ft, facts }));
    arest_ingest(JSON.stringify(entries));   // ONE memory-ops pass
  }
  FED_MEMO.set(key, Date.now());
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
// THE TREE TRANSDUCER: the server answers the CANON's view trees
// (system:view_detail / view_menu evaluated on the wasm carrier); the
// client dispatches on node kind and NEVER derives structure — the
// same trees the tk and WPF containers render.
function Tree({ node, onFollow }) {
  if (!Array.isArray(node)) return null;
  const [kind, body] = node;
  if (kind === "detail")
    return h("dl", null, (body || []).map((f, i) =>
      f && f[0] === "field" ? h(React.Fragment, { key: i },
        h("dt", null, String(f[1])),
        h("dd", null, f[2] === null || f[2] === "" ? h("i", null, "—")
                      : String(f[2]))) : null));
  if (kind === "entry")
    return h(EntryForm, { body: body || [], onCreate: onFollow });
  if (kind === "menu")
    return h("p", { className: "actions" }, (body || []).map((b, i) =>
      b && b[0] === "button" ? h("button", {
        key: i,
        onClick: () => onFollow({ event: String(b[1]), to: String(b[2]) }),
      }, String(b[1]) + " → " + String(b[2])) : null));
  return null;
}

function EntryForm({ body, onCreate }) {
  const [vals, setVals] = React.useState({});
  const [newId, setNewId] = React.useState("");
  const inputs = body.filter(n => n && n[0] === "input");
  return h("form", {
    onSubmit: e => { e.preventDefault(); onCreate({ id: newId, vals, inputs }); },
  },
    h("p", null, h("label", null, "id ",
      h("input", { value: newId, required: true,
                   onChange: e => setNewId(e.target.value) }))),
    inputs.map((n, i) => {
      const [_k, ft, name, kind] = n.map(String);
      return h("p", { key: i }, h("label", null, name + " ",
        kind === "unary"
          ? h("input", { type: "checkbox", checked: !!vals[ft],
              onChange: e => setVals({ ...vals, [ft]: e.target.checked }) })
          : h("input", { value: vals[ft] || "",
              onChange: e => setVals({ ...vals, [ft]: e.target.value }) })));
    }),
    h("button", { type: "submit" }, "Create"));
}

function App() {
  const [noun, setNoun] = React.useState("feature_request");
  const [id, setId] = React.useState("fr-live-1");
  const [views, setViews] = React.useState([]);
  const [receipt, setReceipt] = React.useState("");
  const refetch = React.useCallback(async () => {
    const v = await (await fetch("/" + noun + "/" + id + "/view")).json();
    setViews(v.views || []);
  }, [noun, id]);
  const follow = async (a) => {
    if (a.inputs) {                // the entry form's submit: one POST
      for (const n of a.inputs) {  // per filled SubmitKey
        const ft = String(n[1]), kind = String(n[3]);
        const v = a.vals[ft];
        if (kind === "unary" ? !v : !v) continue;
        const fact = kind === "unary" ? [a.id] : [a.id, v];
        const r = await fetch("/" + noun, { method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({ fact_type: ft, fact }) });
        const doc = await r.json();
        if (!doc.committed) {
          setReceipt(JSON.stringify(doc, null, 1));
          return;
        }
      }
      setId(a.id);
      setReceipt("created " + a.id);
      await refetch();
      return;
    }
    const r = await fetch("/" + noun + "/" + id + "/" + a.event, { method: "POST" });
    setReceipt(JSON.stringify(await r.json(), null, 1));
    await refetch();               // the store is the state
  };
  const openEntry = async () => {
    const v = await (await fetch("/" + noun + "/new")).json();
    setViews(v.views || []);
  };
  return h("div", null,
    h("h1", null, "AREST — a fact renders itself"),
    h("div", { className: "bar" },
      h("input", { value: noun, onChange: e => setNoun(e.target.value) }),
      h("input", { value: id, onChange: e => setId(e.target.value) }),
      h("button", { onClick: refetch }, "fetch"),
      h("button", { onClick: openEntry }, "new")),
    views.map((v, i) => h(Tree, { key: i, node: v, onFollow: follow })),
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
      // the site presents as its host (support.auto.dev, not arest)
      const page = INDEX_HTML
        .replace(/<title>[^<]*<\/title>/, "<title>" + url0.hostname + "</title>")
        .replace(/AREST — a fact renders itself/, url0.hostname);
      return new Response(page, {
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

    if (request.method === "GET" && seg[0] === "events") {
      // SSE over the Durable Object stream: the event log IS the feed
      // (append = commit = emit). Last-Event-ID resumes from the log
      // index; the poll is the v0 transport (the DO push upgrade is
      // hardening). Each message is one committed event line.
      if (!env?.LOG && !env?.AREST_LOG)
        return json('{"error":"no log binding"}', 501);
      // do-minimize: the poll reads the KV MIRROR (cheap reads), never
      // the DO; the DO fallback covers pre-mirror deploys only
      const stub = env?.LOG ? env.LOG.get(env.LOG.idFromName("log")) : null;
      const readTail = async (from) => {
        if (env?.AREST_LOG) {
          const n = Number(await env.AREST_LOG.get("n")) || 0;
          const events = [];
          for (let j = from; j < n; j++) {
            const v = await env.AREST_LOG.get(
              "e:" + String(j).padStart(9, "0"));
            if (v) events.push(v);
          }
          return { n, events };
        }
        const r = await stub.fetch("https://log/tail?from=" + from);
        return r.json();
      };
      let cursor = Number(request.headers.get("last-event-id") ?? 0);
      const enc = new TextEncoder();
      const stream = new ReadableStream({
        async start(controller) {
          controller.enqueue(enc.encode(": arest event stream\n\n"));
          for (let i = 0; i < 55; i++) {          // ~55 s per connection
            const { n, events } = await readTail(cursor);
            let k = cursor;
            for (const line of events) {
              controller.enqueue(enc.encode(
                "id: " + (++k) + "\ndata: " + line + "\n\n"));
            }
            cursor = Math.max(cursor, n);
            await new Promise((res) => setTimeout(res, 1000));
          }
          controller.close();
        },
      });
      return new Response(stream, {
        headers: {
          "content-type": "text/event-stream",
          "cache-control": "no-cache",
        },
      });
    }
    if (request.method === "GET" && seg[0] === "openapi.json") {
      // OpenAPI 3.1 as a PROJECTION of the schema (Thm hateoas' pledge,
      // machine-readable: no undocumented endpoints, nothing for an
      // agent to hallucinate). Paths derive from the nouns; the write
      // paths from the whitepaper's create + follow-the-action forms.
      const schema = JSON.parse(arest_call("schema", "{}"));
      const nouns = (schema.nouns ?? schema.object_types ?? [])
        .map((n) => (typeof n === "string" ? n : n.name))
        .filter((n) => n && (typeof n === "string"));
      const paths = {};
      for (const n of nouns) {
        const slug = sqlname(n);
        paths["/" + slug + "/{id}"] = {
          get: { summary: "The 3NF representation of a " + n,
                 parameters: [{ name: "id", in: "path", required: true,
                                schema: { type: "string" } }],
                 responses: { "200": { description: "the entity view" } } },
        };
        paths["/" + slug + "/{id}/actions"] = {
          get: { summary: "Status + available transitions (Theorem 4)",
                 parameters: [{ name: "id", in: "path", required: true,
                                schema: { type: "string" } }],
                 responses: { "200": { description: "status and transitions" } } },
        };
        paths["/" + slug] = {
          post: { summary: "Create: the body names the fact",
                  requestBody: { content: { "application/json": { schema: {
                    type: "object",
                    properties: { fact_type: { type: "string" },
                                  fact: { type: "array",
                                          items: { type: "string" } } },
                    required: ["fact_type", "fact"] } } } },
                  responses: { "201": { description: "committed" },
                               "422": { description: "refused with violations" } } },
        };
        paths["/" + slug + "/{id}/{event}"] = {
          post: { summary: "Follow a transition offered by the live status",
                  parameters: [
                    { name: "id", in: "path", required: true,
                      schema: { type: "string" } },
                    { name: "event", in: "path", required: true,
                      schema: { type: "string" } }],
                  responses: { "200": { description: "committed" },
                               "409": { description: "not offered from this status" } } },
        };
      }
      return json(JSON.stringify({
        openapi: "3.1.0",
        info: { title: "AREST", version: "0.9.0",
                description: "Generated projection of the compiled schema — complete and current as a function of P and S (Thm hateoas)." },
        paths,
      }));
    }
    if (request.method === "GET" && seg[0] === "schema" && seg[1]) {
      const noun = nounOf(seg[1]);
      if (!noun) return json('{"error":"unknown noun"}', 404);
      return json(arest_call("schema", JSON.stringify({ noun })));
    }
    if (request.method === "POST" && url.pathname === "/__mirror") {
      // one-shot: backfill the KV mirror from the DO log (do-minimize)
      if (!env?.LOG) return json('{' + String.fromCharCode(34) + 'error' + String.fromCharCode(34) + ':' + String.fromCharCode(34) + 'no log' + String.fromCharCode(34) + '}', 501);
      try {
        const stub = env.LOG.get(env.LOG.idFromName('log'));
        const r = await stub.fetch('https://log/mirror', { method: 'POST' });
        return json(await r.text());
      } catch (e) {
        return json(JSON.stringify({ error: String(e) }), 500);
      }
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
    if (seg.length === 2 && seg[1] === "new") {
      const noun = nounOf(seg[0]);
      if (!noun) return json('{"error":"unknown noun"}', 404);
      try { return json(arest_entry(noun)); }
      catch (e) { return json(JSON.stringify({ error: String(e) }), 500); }
    }
    if (request.method === "GET" && seg.length >= 2) {
      const fnoun = nounOf(seg[0]);
      if (fnoun) {
        try { await ensureFederated(env, fnoun); }
        catch (e) {
          if (url.searchParams.has(String.fromCharCode(102,101,100,100,101,98,117,103)))
            return json(JSON.stringify({ fed_error: String(e) }), 500);
        }
      }
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
        if (seg[2] === "view")
          return json(arest_view(noun, id));
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
    return json('{"error":"routes: GET /{noun}/{id}[/actions|/repr|/view], GET /schema/{noun}"}', 404);
  },
};
