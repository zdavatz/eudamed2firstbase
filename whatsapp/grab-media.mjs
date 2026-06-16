#!/usr/bin/env node
// Reconnect auth and download target media messages inline as they appear
// in any history.set / upsert batch. No QR if already paired. If --repair is
// passed, wipes auth first to force a fresh pairing + full history resync
// (QR printed to qr-sync.png).
//
// Usage: node grab-media.mjs [--repair] <jid> <id1> [id2 ...]

import makeWASocket, {
  useMultiFileAuthState,
  makeCacheableSignalKeyStore,
  fetchLatestBaileysVersion,
  downloadMediaMessage,
  DisconnectReason,
} from "@whiskeysockets/baileys";
import QRCode from "qrcode";
import qrcodeTerminal from "qrcode-terminal";
import pino from "pino";
import { resolve, dirname } from "path";
import { fileURLToPath } from "url";
import { mkdirSync, writeFileSync, rmSync, readFileSync } from "fs";

const __dirname = dirname(fileURLToPath(import.meta.url));
const AUTH_DIR = resolve(__dirname, "auth");
const OUT_DIR = resolve(__dirname, "attachments");
const QR_PNG = resolve(__dirname, "qr-sync.png");
const logger = pino({ level: "silent" });

let argv = process.argv.slice(2);
const repair = argv.includes("--repair");
argv = argv.filter((a) => a !== "--repair");
const [jid, ...wantIds] = argv;
if (!jid || wantIds.length === 0) {
  console.error("Usage: node grab-media.mjs [--repair] <jid> <id1> [id2 ...]");
  process.exit(1);
}
const wanted = new Set(wantIds);
mkdirSync(OUT_DIR, { recursive: true });
if (repair) { try { rmSync(AUTH_DIR, { recursive: true, force: true }); console.error("[repair] wiped auth"); } catch {} }

// Anchor for on-demand history: newest known message in this chat. fetchMessageHistory
// pulls messages OLDER than the anchor, so anchoring on the newest message delivers
// the (older) target attachments via a messaging-history.set onDemand batch.
const NDJSON = resolve(__dirname, "messages.ndjson");
let anchor = null;
try {
  for (const l of readFileSync(NDJSON, "utf8").split("\n")) {
    if (!l) continue;
    let m; try { m = JSON.parse(l); } catch { continue; }
    if (m.jid !== jid || !m.ts) continue;
    if (!anchor || m.ts > anchor.ts) anchor = { key: { remoteJid: jid, id: m.id, fromMe: !!m.fromMe }, ts: m.ts };
  }
} catch {}

function docOf(msg) {
  return (
    msg.message?.documentMessage ||
    msg.message?.documentWithCaptionMessage?.message?.documentMessage ||
    msg.message?.imageMessage ||
    msg.message?.videoMessage ||
    msg.message?.audioMessage ||
    null
  );
}

let saved = 0;
async function maybeDownload(sock, m) {
  const id = m.key?.id;
  if (!id || !wanted.has(id)) return;
  const d = docOf(m);
  const fname = d?.fileName || `${id}.${(d?.mimetype || "bin").split("/")[1] || "bin"}`;
  try {
    const buf = await downloadMediaMessage(m, "buffer", {}, { logger, reuploadRequest: sock.updateMediaMessage });
    writeFileSync(resolve(OUT_DIR, fname), buf);
    console.error(`[saved] ${fname} (${buf.length} bytes)`);
    wanted.delete(id);
    saved++;
    if (wanted.size === 0) { console.error(`[done] ${saved} saved`); sock.end(); setTimeout(() => process.exit(0), 400); }
  } catch (e) {
    console.error(`[download failed] ${fname}: ${e?.message || e}`);
  }
}

async function start() {
  const { state, saveCreds } = await useMultiFileAuthState(AUTH_DIR);
  const { version } = await fetchLatestBaileysVersion();
  const sock = makeWASocket({
    version, logger,
    auth: { creds: state.creds, keys: makeCacheableSignalKeyStore(state.keys, logger) },
    browser: ["eudamed2firstbase", "CLI", "1.0"],
    syncFullHistory: true,
    markOnlineOnConnect: true,
  });
  sock.ev.on("creds.update", saveCreds);

  sock.ev.on("messaging-history.set", async ({ messages, syncType, progress }) => {
    const hit = (messages || []).filter((m) => wanted.has(m.key?.id)).length;
    if (messages?.length) console.error(`[history.set] n=${messages.length} syncType=${syncType} progress=${progress} hits=${hit}`);
    for (const m of messages || []) await maybeDownload(sock, m);
  });
  sock.ev.on("messages.upsert", async ({ messages }) => {
    for (const m of messages || []) await maybeDownload(sock, m);
  });

  sock.ev.on("connection.update", async (u) => {
    const { connection, qr, lastDisconnect } = u;
    if (qr) {
      await QRCode.toFile(QR_PNG, qr, { width: 480, margin: 2 });
      console.error(`[qr] ${QR_PNG} — scan with WhatsApp > Linked Devices`);
      qrcodeTerminal.generate(qr, { small: true });
    }
    if (connection === "open") {
      console.error("[connected, online] waiting for history / media…");
      // Actively pull older history anchored on the newest known message — passive
      // history.set does not re-deliver old messages on a reconnect.
      if (anchor?.key) {
        setTimeout(async () => {
          if (wanted.size === 0) return;
          try {
            console.error(`[fetchMessageHistory] requesting 80 older than ${anchor.key.id}…`);
            await sock.fetchMessageHistory(80, anchor.key, anchor.ts);
          } catch (e) {
            console.error("[fetchMessageHistory failed]", e?.message || e);
          }
        }, 6000);
      }
      setTimeout(() => { console.error(`[timeout] saved ${saved}, missing ${[...wanted].join(",") || "none"}`); sock.end(); process.exit(saved > 0 ? 0 : 3); }, repair ? 120000 : 70000);
    }
    if (connection === "close") {
      const code = lastDisconnect?.error?.output?.statusCode;
      console.error(`[close] code=${code}`);
      if (code === DisconnectReason.loggedOut) process.exit(2);
      if (code === 515) setTimeout(() => start().catch(() => process.exit(1)), 1500);
    }
  });
}
start().catch((e) => { console.error("fatal:", e?.message || e); process.exit(1); });
