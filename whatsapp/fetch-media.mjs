#!/usr/bin/env node
// Download specific media messages from a chat by re-fetching history protos
// (which carry mediaKey/directPath) and decrypting them. Reuses ./auth
// (already paired) — no QR. Saves decrypted files to ./attachments/.
//
// Usage: node fetch-media.mjs <jid> <anchorId> <anchorTs> <fromMe 0|1> <id1> [id2 ...]

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
import { mkdirSync, writeFileSync } from "fs";

const __dirname = dirname(fileURLToPath(import.meta.url));
const AUTH_DIR = resolve(__dirname, "auth");
const OUT_DIR = resolve(__dirname, "attachments");
const logger = pino({ level: "silent" });

const [, , jid, anchorId, anchorTsRaw, fromMeRaw, ...wantIds] = process.argv;
if (!jid || !anchorId || wantIds.length === 0) {
  console.error("Usage: node fetch-media.mjs <jid> <anchorId> <anchorTs> <fromMe 0|1> <id1> [id2 ...]");
  process.exit(1);
}
const anchorTs = parseInt(anchorTsRaw, 10);
const wanted = new Set(wantIds);
mkdirSync(OUT_DIR, { recursive: true });

const byId = new Map(); // id -> full WAMessage
let requested = 0;
let saved = 0;

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

async function start() {
  const { state, saveCreds } = await useMultiFileAuthState(AUTH_DIR);
  const { version } = await fetchLatestBaileysVersion();

  const sock = makeWASocket({
    version,
    logger,
    auth: { creds: state.creds, keys: makeCacheableSignalKeyStore(state.keys, logger) },
    browser: ["eudamed2firstbase", "CLI", "1.0"],
    syncFullHistory: true,
    markOnlineOnConnect: false,
  });

  sock.ev.on("creds.update", saveCreds);

  async function tryDownloads() {
    for (const id of [...wanted]) {
      const msg = byId.get(id);
      if (!msg) continue;
      const d = docOf(msg);
      const fname = d?.fileName || `${id}.${(d?.mimetype || "bin").split("/")[1] || "bin"}`;
      try {
        const buf = await downloadMediaMessage(
          msg,
          "buffer",
          {},
          { logger, reuploadRequest: sock.updateMediaMessage }
        );
        const dest = resolve(OUT_DIR, fname);
        writeFileSync(dest, buf);
        console.error(`[saved] ${fname} (${buf.length} bytes) -> ${dest}`);
        wanted.delete(id);
        saved++;
      } catch (e) {
        console.error(`[download failed] ${id} ${fname}: ${e?.message || e}`);
      }
    }
    if (wanted.size === 0) {
      console.error(`[done] ${saved} file(s) saved`);
      sock.end();
      setTimeout(() => process.exit(0), 400);
    }
  }

  sock.ev.on("messaging-history.set", async ({ messages, syncType }) => {
    let hit = 0;
    (messages || []).forEach((m) => {
      const id = m.key?.id;
      if (id) byId.set(id, m);
      if (wanted.has(id)) hit++;
    });
    console.error(`[history.set] n=${messages?.length || 0} syncType=${syncType} wantedHits=${hit}`);
    if (hit > 0) await tryDownloads();
  });

  sock.ev.on("connection.update", async (u) => {
    const { connection, lastDisconnect } = u;
    if (connection === "open") {
      console.error("[connected] requesting on-demand history…");
      // pull older history anchored at the newest known message
      const anchorKey = { remoteJid: jid, id: anchorId, fromMe: fromMeRaw === "1" };
      const attempt = async () => {
        if (requested >= 4 || wanted.size === 0) return;
        requested++;
        try {
          console.error(`[fetchMessageHistory] attempt ${requested} (50 older than anchor)`);
          await sock.fetchMessageHistory(50, anchorKey, anchorTs);
        } catch (e) {
          console.error("[fetchMessageHistory failed]", e?.message || e);
        }
        setTimeout(attempt, 8000);
      };
      attempt();
      // also try downloads in case ids already present, and hard timeout
      setTimeout(tryDownloads, 5000);
      setTimeout(() => { console.error(`[timeout] saved ${saved}, still missing: ${[...wanted].join(",")}`); sock.end(); process.exit(saved > 0 ? 0 : 3); }, 60000);
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
