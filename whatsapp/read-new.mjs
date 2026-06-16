#!/usr/bin/env node
// Reconnect auth and print/append any messages from one jid that arrive
// via offline-delivery (messages.upsert notify) or history.set, downloading
// attachments inline. No QR if already paired.
//
// Usage: node read-new.mjs <jid> [sinceEpochSeconds]

import makeWASocket, {
  useMultiFileAuthState,
  makeCacheableSignalKeyStore,
  fetchLatestBaileysVersion,
  downloadMediaMessage,
  DisconnectReason,
} from "@whiskeysockets/baileys";
import pino from "pino";
import { resolve, dirname } from "path";
import { fileURLToPath } from "url";
import { mkdirSync, writeFileSync, appendFileSync } from "fs";

const __dirname = dirname(fileURLToPath(import.meta.url));
const AUTH_DIR = resolve(__dirname, "auth");
const OUT_DIR = resolve(__dirname, "attachments");
const NDJSON = resolve(__dirname, "messages.ndjson");
const logger = pino({ level: "silent" });

const [jid, sinceArg] = process.argv.slice(2);
if (!jid) { console.error("Usage: node read-new.mjs <jid> [sinceEpochSeconds]"); process.exit(1); }
const since = sinceArg ? parseInt(sinceArg, 10) : 0;
mkdirSync(OUT_DIR, { recursive: true });

const seen = new Set();

function mediaOf(msg) {
  const m = msg.message || {};
  return (
    m.documentMessage ||
    m.documentWithCaptionMessage?.message?.documentMessage ||
    m.imageMessage ||
    m.videoMessage ||
    m.audioMessage ||
    null
  );
}
function textOf(msg) {
  const m = msg.message || {};
  return (
    m.conversation ||
    m.extendedTextMessage?.text ||
    m.imageMessage?.caption ||
    m.documentWithCaptionMessage?.message?.documentMessage?.caption ||
    m.documentMessage?.caption ||
    ""
  );
}

async function handle(sock, m) {
  if (!m?.key) return;
  const mjid = m.key.remoteJid;
  if (mjid !== jid) return;
  const id = m.key.id;
  if (seen.has(id)) return;
  const ts = Number(m.messageTimestamp || 0);
  if (since && ts && ts < since) return;
  seen.add(id);
  const iso = ts ? new Date(ts * 1000).toISOString() : "?";
  const med = mediaOf(m);
  const txt = textOf(m);
  if (med) {
    const fname = med.fileName || `${id}.${(med.mimetype || "bin").split("/")[1] || "bin"}`;
    let note = "";
    try {
      const buf = await downloadMediaMessage(m, "buffer", {}, { logger, reuploadRequest: sock.updateMediaMessage });
      writeFileSync(resolve(OUT_DIR, fname), buf);
      note = `saved ${buf.length}B`;
    } catch (e) { note = `download failed: ${e?.message || e}`; }
    console.error(`[${iso}] ${m.key.fromMe ? "ME" : "MAIK"} [ATTACH] ${fname} — ${note}${txt ? " | caption: " + txt : ""}`);
  } else if (txt) {
    console.error(`[${iso}] ${m.key.fromMe ? "ME" : "MAIK"} ${txt.replace(/\n/g, " ⏎ ")}`);
  }
  appendFileSync(NDJSON, JSON.stringify({ jid: mjid, id, ts, iso, fromMe: !!m.key.fromMe, type: med ? "media" : "text", text: txt, media: med ? (med.fileName || med.mimetype) : null, source: "read-new" }) + "\n");
}

async function start() {
  const { state, saveCreds } = await useMultiFileAuthState(AUTH_DIR);
  const { version } = await fetchLatestBaileysVersion();
  const sock = makeWASocket({
    version, logger,
    auth: { creds: state.creds, keys: makeCacheableSignalKeyStore(state.keys, logger) },
    browser: ["eudamed2firstbase", "CLI", "1.0"],
    markOnlineOnConnect: true,
  });
  sock.ev.on("creds.update", saveCreds);
  sock.ev.on("messaging-history.set", async ({ messages }) => { for (const m of messages || []) await handle(sock, m); });
  sock.ev.on("messages.upsert", async ({ messages }) => { for (const m of messages || []) await handle(sock, m); });
  sock.ev.on("connection.update", async (u) => {
    const { connection, lastDisconnect } = u;
    if (connection === "open") {
      console.error("[connected] waiting for offline delivery…");
      setTimeout(() => { console.error("[done]"); sock.end(); process.exit(0); }, 40000);
    }
    if (connection === "close") {
      const code = lastDisconnect?.error?.output?.statusCode;
      if (code === 515) return setTimeout(() => start().catch(() => process.exit(1)), 1500);
      console.error(`[close] code=${code}`); process.exit(code === DisconnectReason.loggedOut ? 2 : 0);
    }
  });
}
start().catch((e) => { console.error("fatal:", e?.message || e); process.exit(1); });
