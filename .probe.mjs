const list = await (await fetch("http://127.0.0.1:9223/json")).json();
const page = list.find(t => t.type === "page" && t.url.includes("1421"));
const ws = new WebSocket(page.webSocketDebuggerUrl);
let id = 0; const pending = new Map();
ws.onmessage = e => { const m = JSON.parse(e.data); if (m.id && pending.has(m.id)) { pending.get(m.id)(m.result); pending.delete(m.id); } };
await new Promise(r => ws.onopen = r);
const send = (method, params) => new Promise(res => { const i = ++id; pending.set(i, res); ws.send(JSON.stringify({ id: i, method, params })); });
const ev = async expr => (await send("Runtime.evaluate", { expression: expr, returnByValue: true })).result?.value;
const out = await ev(`(() => {
  const hits = [];
  document.querySelectorAll('*').forEach(el => {
    const t = (el.textContent || '');
    if (el.children.length === 0 && /mock/i.test(t)) {
      hits.push({ text: t.trim().slice(0, 40), cls: el.className, parentCls: el.parentElement?.className });
    }
  });
  return JSON.stringify(hits, null, 1);
})()`);
console.log(out);
ws.close();
