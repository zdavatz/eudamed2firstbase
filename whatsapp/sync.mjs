#!/usr/bin/env node
// WhatsApp history sync — pair a (separate) linked device, pull full history
// and append every message to messages.ndjson. Read-only w.r.t. chats: it
// only sends the unavoidable delivery receipts + the QR pairing handshake.
//
// Auth lives in ./auth-sync (independent of ./auth used for sending), so the
// existing send session is untouched.
//
// Usage: node sync.mjs            # pair (QR) + sync, keeps running
//        node sync.mjs --seconds 600   # auto-exit after N seconds idle-capped
//
// QR is written to ./qr-sync.png (scan with WhatsApp > Linked Devices) and
// also printed as ASCII to the terminal.

import makeWASocket, {
  useMultiFileAuthState,
  makeCacheableSignalKeyStore,
  fetchLatestBaileysVersion,
  DisconnectReason,
} from "@whiskeysockets/baileys";
import qrcodeTerminal from "qrcode-terminal";
import QRCode from "qrcode";
import pino from "pino";
import { resolve, dirname } from "path";
import { fileURLToPath } from "url";
import { appendFileSync, writeFileSync } from "fs";

const __dirname = dirname(fileURLToPath(import.meta.url));
const AUTH_DIR = resolve(__dirname, "auth-sync");
const STORE = resolve(__dirname, "messages.ndjson");
const QR_PNG = resolve(__dirname, "qr-sync.png");
const logger = pino({ level: "silent" });

const secsArg = process.argv.indexOf("--seconds");
const HARD_SECONDS = secsArg > -1 ? parseInt(process.argv[secsArg + 1] || "0", 10) : 0;

function describe(m) {
  const msg = m.message || {};
  let type = "text";
  let text = msg.conversation || msg.extendedTextMessage?.text || "";
  let media = null;
  const wrap = msg.documentWithCaptionMessage?.message?.documentMessage;
  if (msg.imageMessage) { type = "image"; text = msg.imageMessage.caption || ""; media = { mimetype: msg.imageMessage.mimetype, fileLength: Number(msg.imageMessage.fileLength) || 0 }; }
  else if (msg.videoMessage) { type = "video"; text = msg.videoMessage.caption || ""; media = { mimetype: msg.videoMessage.mimetype, fileLength: Number(msg.videoMessage.fileLength) || 0, seconds: msg.videoMessage.seconds }; }
  else if (msg.documentMessage) { type = "document"; text = msg.documentMessage.caption || ""; media = { fileName: msg.documentMessage.fileName, mimetype: msg.documentMessage.mimetype, fileLength: Number(msg.documentMessage.fileLength) || 0 }; }
  else if (wrap) { type = "document"; text = wrap.caption || ""; media = { fileName: wrap.fileName, mimetype: wrap.mimetype, fileLength: Number(wrap.fileLength) || 0 }; }
  else if (msg.audioMessage) { type = "audio"; media = { mimetype: msg.audioMessage.mimetype, seconds: msg.audioMessage.seconds, ptt: !!msg.audioMessage.ptt }; }
  else if (msg.stickerMessage) { type = "sticker"; }
  else if (msg.reactionMessage) { type = "reaction"; text = msg.reactionMessage.text || ""; }
  else if (msg.locationMessage) { type = "location"; text = `${msg.locationMessage.degreesLatitude},${msg.locationMessage.degreesLongitude}`; }
  else if (msg.contactMessage) { type = "contact"; text = msg.contactMessage.displayName || ""; }
  return { type, text, media };
}

let written = 0;
const seen = new Set();
function store(m, source) {
  const jid = m.key?.remoteJid;
  if (!jid || jid === "status@broadcast") return;
  const id = m.key?.id || "";
  const dedupe = `${jid}|${id}`;
  if (seen.has(dedupe)) return;
  seen.add(dedupe);
  const ts = Number(m.messageTimestamp) || 0;
  const d = describe(m);
  const row = {
    jid,
    id,
    ts,
    iso: ts ? new Date(ts * 1000).toISOString() : "",
    fromMe: !!m.key?.fromMe,
    pushName: m.pushName || "",
    type: d.type,
    text: d.text,
    media: d.media,
    source,
  };
  appendFileSync(STORE, JSON.stringify(row) + "\n");
  written++;
}

async function start() {
  const { state, saveCreds } = await useMultiFileAuthState(AUTH_DIR);
  const { version } = await fetchLatestBaileysVersion();
  console.error(`[sync] WA version ${version.join(".")} — store: ${STORE}`);

  const sock = makeWASocket({
    version,
    logger,
    auth: { creds: state.creds, keys: makeCacheableSignalKeyStore(state.keys, logger) },
    browser: ["eudamed2firstbase-sync", "CLI", "1.0"],
    syncFullHistory: true,
    markOnlineOnConnect: false,
  });

  sock.ev.on("creds.update", saveCreds);

  sock.ev.on("messaging-history.set", ({ messages, progress, syncType }) => {
    (messages || []).forEach((m) => store(m, "history"));
    console.error(`[history.set] +${messages?.length || 0} (total ${written}) progress=${progress} syncType=${syncType}`);
  });

  sock.ev.on("messages.upsert", ({ messages, type }) => {
    (messages || []).forEach((m) => store(m, `upsert:${type}`));
  });

  sock.ev.on("connection.update", async (u) => {
    const { connection, qr, lastDisconnect } = u;
    if (qr) {
      try {
        await QRCode.toFile(QR_PNG, qr, { width: 480, margin: 2 });
        console.error(`[qr] PNG written: ${QR_PNG} — scan with WhatsApp > Linked Devices`);
      } catch (e) {
        console.error("[qr] PNG render failed:", e?.message || e);
      }
      qrcodeTerminal.generate(qr, { small: true });
    }
    if (connection === "open") {
      console.error("[connected] full-history sync running… (leave this process running)");
      if (HARD_SECONDS > 0) setTimeout(() => { console.error("[exit] hard timeout"); process.exit(0); }, HARD_SECONDS * 1000);
    }
    if (connection === "close") {
      const code = lastDisconnect?.error?.output?.statusCode;
      console.error(`[close] code=${code}`);
      if (code === DisconnectReason.loggedOut) {
        console.error("[loggedOut] auth invalid — delete auth-sync/ and re-pair");
        process.exit(2);
      }
      // 515 restartRequired (after first pair) or transient → reconnect
      setTimeout(() => start().catch((e) => { console.error("restart failed:", e?.message); process.exit(1); }), 1500);
    }
  });
}

// ensure store file exists
writeFileSync(STORE, "", { flag: "a" });
start().catch((e) => { console.error("fatal:", e?.message || e); process.exit(1); });
