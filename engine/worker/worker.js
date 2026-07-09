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
//                                 fact (the write path: commit appends
//                                 to the D1 event stream)
import init, { arest_load, arest_call, arest_apply, arest_view, arest_entry, arest_ingest } from "./arest_core.js";
import wasmModule from "./arest_core_bg.wasm";
import SIDECAR from "./sidecar.js";

const PLURAL = {}; // e.g. { order: "orders" } — inverse applied on route parse

// The event log: Def. iso's single serialized append per app stream,
// D1 edition — the events table's single-writer SQLite serializes
// commits (INSERT..RETURNING mints n atomically), the KV mirror is
// the read path (SSE + boot), the stream is the store of record. The
// isolate's wasm store is the snapshot (the bundled sidecar) plus
// this log's tail, replayed at boot exactly like the resident's
// stream-watermark discipline. (The ArestLog Durable Object held the
// commit role until 2026-07-09; retired via migration v2 once a
// production write proved the D1 append.)

let ready = null;
async function ensure(env) {
  if (!ready) {
    // a failed boot must NOT poison the isolate: a cached REJECTED
    // promise turns every later request into an instant 1101 (the
    // 2026-07-08 flap — 200s and 1101s alternating by isolate).
    // Reset on failure so the next request retries the boot.
    ready = (async () => {
      await init(wasmModule);
      const sidecar =
        env?.SIDECAR_JSON ?? (env?.STORE ? await env.STORE.get("sidecar") : null) ?? SIDECAR;
      arest_load(sidecar);
      // replay the log tail over the snapshot (a replayed duplicate
      // reads as refused — D' == D — and changes nothing). The KV
      // mirror is a MEMO of a restriction over the D1 stream (Prop.
      // derive: every served value is a rho-application over the
      // record; Cor. middleware's note: a cache's policy is denoted
      // inside the system, so the memo is never authoritative and
      // never hand-repaired). Boot walks it by the stream's own index;
      // any structural miss FALSIFIES the memo and the whole tail
      // re-derives from D1.
      let events = null;
      if (env?.AREST_LOG) {
        const n = Number(await env.AREST_LOG.get("n"));
        if (Number.isFinite(n) && n > 0) {
          events = [];
          for (let j = 1; j <= n; j++) {
            const v = await env.AREST_LOG.get(
              "e:" + String(j).padStart(9, "0"));
            if (v === null) { events = null; break; }
            events.push(v);
          }
        }
      }
      if (events === null && env?.DB) {
        // the D1 stream is the record (Def. iso with ZERO DOs: the
        // single-writer SQLite serializes appends; n orders the
        // replay). Re-deriving the memo writes each line at the
        // stream's own n and drops keys the derivation does not yield
        // (compaction that preserves the population rho observes).
        const q = await env.DB.prepare(
          "SELECT n, line FROM events WHERE app = ?1 ORDER BY n"
        ).bind(APP_STREAM).all();
        const rows = q.results ?? [];
        events = rows.map((r) => r.line);
        if (env?.AREST_LOG && rows.length) {
          const want = new Set(["n"]);
          for (const r of rows) {
            const k = "e:" + String(r.n).padStart(9, "0");
            want.add(k);
            await env.AREST_LOG.put(k, r.line);
          }
          const listed = await env.AREST_LOG.list({ prefix: "e:" });
          for (const k of listed.keys) {
            if (!want.has(k.name)) await env.AREST_LOG.delete(k.name);
          }
          await env.AREST_LOG.put("n", String(rows[rows.length - 1].n));
        }
      }
      for (const line of events ?? []) {
        try {
          const e = JSON.parse(line);
          arest_apply(JSON.stringify({ fact_type: e.ft, fact: e.fact }));
        } catch {}
      }
    })().catch((e) => {
      ready = null;
      throw e;
    });
  }
  return ready;
}

// ---- the federated connector (contact-federation.md at the edge) ----
// A noun backed by an External System refreshes on read: fetch the
// system's URL+URI with the AREST_SECRET_<SYSTEM> credential, map
// columns to <Noun>_has_<Field> facts, ingest ISOLATE-LOCALLY via
// arest_apply (no DO appends: federated rows are refetchable cache,
// not commits). OWA: no credential or unreachable = empty population.
// the app's stream key in the D1 events table (one worker serves one
// app today; multi-tenant streams key by app)
const APP_STREAM = "support.auto.dev";

// FEDERATED VERIFICATION (the dissolved auth gate, 2026-07-08): the
// worker PRESENTS the caller's credential to auth.vin — the session
// cookie or 'Authorization: users API-Key <k>' forwards to
// GET /api/users/me — and the answered identity IS the actor. NO
// secrets live here; identity is demand-driven federated data,
// memoized ~60s per isolate (apis' AUTH_MEMO precedent). Anonymous
// answers null (OWA — routes decide what anonymity may do).
const AUTH_MEMO = new Map();
const AUTH_TTL = 60_000;

async function verifyActor(request) {
  const auth = request.headers.get("Authorization") ?? "";
  const cookie = request.headers.get("Cookie") ?? "";
  if (!auth && !cookie) return null;
  const key = auth || ("c:" + cookie);
  const hit = AUTH_MEMO.get(key);
  if (hit && Date.now() - hit.t < AUTH_TTL) return hit.actor;
  let actor = null;
  try {
    const headers = {};
    if (auth) headers["Authorization"] = auth;
    if (cookie) headers["Cookie"] = cookie;
    const r = await fetch("https://auth.vin/api/users/me", { headers });
    if (r.ok) {
      const u = await r.json();
      const user = u?.user ?? u ?? {};
      const email = user?.email ?? null;
      if (email) {
        actor = String(email);
        // THE IDENTITY MINT: the caller's own answer federates as
        // facts (isolate-local, never logged) — the DECLARED
        // subscription link anchors the customer policy rules
        const sub = user?.subscription;
        if (sub) {
          try {
            arest_ingest(JSON.stringify([{
              ft: "Subscription_belongs_to_Customer",
              facts: [[String(sub), actor]],
            }]));
          } catch {}
        }
      }
    }
  } catch {}
  AUTH_MEMO.set(key, { t: Date.now(), actor });
  return actor;
}

// ONE serialized append (Def. iso, ZERO DOs): the INSERT mints n
// atomically in a single statement — D1's SQLite single-writer is the
// serializer at the constrained key. The stream's n is the ONE
// identity end to end: the D1 row, the mirror key e:{n}, and the SSE
// event id are the same number (the DO era kept a second, 0-indexed
// numbering for the mirror — the off-by-one class died with it). The
// KV write-through keeps the memo current for SSE + boot.
async function appendEvent(env, line) {
  let n = null;
  if (env?.DB) {
    const row = await env.DB.prepare(
      "INSERT INTO events (app, n, line) VALUES (?1, " +
      "(SELECT COALESCE(MAX(n), 0) + 1 FROM events WHERE app = ?1), ?2) " +
      "RETURNING n"
    ).bind(APP_STREAM, line).first();
    n = row?.n ?? null;
  }
  if (n !== null && env?.AREST_LOG) {
    await env.AREST_LOG.put("e:" + String(n).padStart(9, "0"), line);
    await env.AREST_LOG.put("n", String(n));
  }
  return n;
}

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
  // another noun gets one same-id bridge row per fetched entity. The
  // FIELD projection rides the boundary too — the canon re-key rules
  // (contact-derivation.md) are the SPEC this table mirrors, because
  // the wasm derive op is the interpretive class and blew the Worker
  // CPU budget both bare and changed-scoped (1101 x2, 2026-07-08;
  // rolled back twice). When the native rules path lands, this table
  // retires and the derive call replaces it.
  const surfaced = popRows("Noun_is_surfaced_as_Noun")
    .find((r) => r[0] === noun);
  if (surfaced && ids.length) {
    const sid = surfaced[1].replace(/[^0-9A-Za-z]+/g, "_").replace(/^_+|_+$/g, "");
    byFt.set(sid + "_is_" + nid, ids.map((rid) => [rid, rid]));
    if (noun === "Contact Submission" && surfaced[1] === "Support Request") {
      // mirrors contact-derivation.md's re-keys + constants, one row
      // per submission field that arrived
      const REKEY = [
        ["Body", "Support_Request_has_Description"],
        ["Issue Type", "Support_Request_has_Subject"],
        ["Email Address", "Support_Request_has_Email_Address"],
        ["Submitter Name", "Support_Request_has_contact_Name"],
        ["Company Name", "Support_Request_has_company_Name"],
        ["Issue Type", "Support_Request_has_Category"],
        ["API Reference", "Support_Request_has_API_Reference"],
        ["Date", "Support_Request_occurred_at_Timestamp"],
        ["User Id", "Support_Request_is_for_User"],
      ];
      for (const [src, dft] of REKEY) {
        const rows = byFt.get(nid + "_has_" +
          src.replace(/[^0-9A-Za-z]+/g, "_"));
        if (rows && rows.length) byFt.set(dft, rows.map((r) => [...r]));
      }
      byFt.set("Support_Request_has_Intake_Source",
               ids.map((rid) => [rid, "contact-form"]));
      byFt.set("Support_Request_uses_Streaming_Mode",
               ids.map((rid) => [rid, "non-streaming"]));
      byFt.set("Support_Request_is_with_Agent",
               ids.map((rid) => [rid, "contact-form"]));
    }
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
      // SSE over the event stream: the log IS the feed (append =
      // commit = emit; Cor. stream — a subscriber is a rho-application
      // awaiting its next evaluation, and Last-Event-ID is the
      // restriction's lower bound). The event id IS the stream's n.
      if (!env?.AREST_LOG)
        return json('{"error":"no log binding"}', 501);
      // do-minimize: the poll reads the KV MEMO (cheap reads), never
      // the commit store
      const readTail = async (from) => {
        const n = Number(await env.AREST_LOG.get("n")) || 0;
        const events = [];
        for (let j = from + 1; j <= n; j++) {
          const v = await env.AREST_LOG.get(
            "e:" + String(j).padStart(9, "0"));
          if (v) events.push(v);
        }
        return { n, events };
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
    if (request.method === "POST" && seg.length === 1) {
      // POST /{noun}: the whitepaper's create ("A POST /orders request
      // creates an Order...") — the body names the fact: {fact_type,
      // fact}. Commit appends to the D1 event stream.
      const noun = nounOf(seg[0]);
      if (!noun) return json('{"error":"unknown noun"}', 404);
      let body = null;
      try { body = await request.json(); } catch {}
      if (!body?.fact_type || !Array.isArray(body?.fact))
        return json('{"error":"body needs fact_type and fact"}', 400);
      // the actor threads from federated verification: the identity
      // auth.vin answers for the caller's own credential. Writes stay
      // OPEN for now (the policy-derivation gate is the next rung) —
      // but every committed event carries WHO (replay ignores the
      // extra field; provenance is append-only history)
      const actor = await verifyActor(request);
      // AUTHORIZATION IS FACTS: the derived triples (authorization.md)
      // answer whether THIS actor may 'create' THIS noun. ENFORCED
      // for 'create' (2026-07-09, Samuel: no external users) — an
      // unauthorized create refuses BEFORE apply, verdict on the
      // receipt. The model-level form (the actor entering P as a
      // fact, validate_S refusing) is the endgame once the richer
      // policy rules land; this guard is the per-operation interim.
      const authorized = actor !== null && popRows(
        "User_is_authorized_for_Operation_on_Resource"
      ).some((t) => t.length >= 3 && t[0] === actor
                 && t[1] === "create" && t[2] === noun);
      if (!authorized) {
        return json(JSON.stringify({ receipt: {
          app: "worker", fact_type: body.fact_type, fact: body.fact,
          committed: false, violations: [],
          refused: "unauthorized: " + (actor ?? "anonymous") +
                   " may not 'create' " + noun,
          actor: actor ?? undefined, authorized: false,
        } }), 403);
      }
      const r = JSON.parse(arest_apply(JSON.stringify(
        { fact_type: body.fact_type, fact: body.fact })));
      if (r.receipt?.committed && r.event) {
        const ev = { ...r.event, actor: actor ?? undefined };
        await appendEvent(env, JSON.stringify(ev));
      }
      if (r.receipt) {
        if (actor) r.receipt.actor = actor;
        r.receipt.authorized = authorized;
      }
      return json(JSON.stringify(r), r.receipt?.committed ? 201 : 422);
    }
    if (seg.length === 2 && seg[1] === "new") {
      const noun = nounOf(seg[0]);
      if (!noun) return json('{"error":"unknown noun"}', 404);
      try { return json(arest_entry(noun)); }
      catch (e) { return json(JSON.stringify({ error: String(e) }), 500); }
    }
    if (request.method === "GET" && seg.length === 1 && seg[0] === "nouns") {
      // the store's noun inventory (the native nouns op) — the menu
      // surface the queue and the OS share
      try { return json(arest_call("nouns", "{}")); }
      catch (e) { return json(JSON.stringify({ error: String(e) }), 500); }
    }
    if (request.method === "GET" && seg.length === 1) {
      // the noun's population (the native list op): the queue page's
      // fuel — ids only, one spine pass, federating first so surfaced
      // nouns list their runtime-minted entities too
      const lnoun = nounOf(seg[0]);
      if (lnoun) {
        try {
          await ensureFederated(env, lnoun);
          for (const r of popRows("Noun_is_surfaced_as_Noun")) {
            if (r.length >= 2 && r[1] === lnoun) {
              await ensureFederated(env, r[0]);
            }
          }
        } catch {}
        try {
          return json(arest_call(
            "list", JSON.stringify({ noun: lnoun })));
        } catch (e) {
          return json(JSON.stringify({ error: String(e) }), 500);
        }
      }
    }
    if (request.method === "GET" && seg.length >= 2) {
      const fnoun = nounOf(seg[0]);
      if (fnoun) {
        try {
          await ensureFederated(env, fnoun);
          // isolate locality: a noun SURFACED AS the requested one
          // must federate in THIS isolate too — its fetched rows are
          // where the requested noun's facts come from (the SR view
          // was empty in fresh isolates until the CS fetch ran here)
          for (const r of popRows("Noun_is_surfaced_as_Noun")) {
            if (r.length >= 2 && r[1] === fnoun) {
              await ensureFederated(env, r[0]);
            }
          }
        }
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
        // the same commit discipline as create: the proven appendEvent
        // (this path wrote straight to the DO before the retirement —
        // and would have silently dropped the append without it)
        const actor = await verifyActor(request);
        const r = JSON.parse(
          arest_apply(JSON.stringify({ fact_type: hit.event, fact })));
        if (r.receipt?.committed && r.event) {
          const ev = { ...r.event, actor: actor ?? undefined };
          await appendEvent(env, JSON.stringify(ev));
        }
        if (r.receipt && actor) r.receipt.actor = actor;
        return json(JSON.stringify(r), r.receipt?.committed ? 200 : 422);
      }
    }
    return json('{"error":"routes: GET /{noun}/{id}[/actions|/repr|/view], GET /schema/{noun}"}', 404);
  },
};
