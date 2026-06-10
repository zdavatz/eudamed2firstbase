#!/usr/bin/env node
// Send a plain text message via the auth-sync session.
// Usage: node send-text.mjs <jid-or-number> <textfile>
//   <jid-or-number> : e.g. 4917661063798  or  4917661063798@s.whatsapp.net
//   <textfile>      : path to a UTF-8 file whose contents are sent verbatim

import makeWASocket, {
  useMultiFileAuthState,
  makeCacheableSignalKeyStore,
  fetchLatestBaileysVersion,
  DisconnectReason,
} from "@whiskeysockets/baileys";
import pino from "pino";
import { resolve, dirname } from "path";
import { fileURLToPath } from "url";
import { readFileSync } from "fs";

const __dirname = dirname(fileURLToPath(import.meta.url));
const AUTH_DIR = resolve(__dirname, "auth-sync");
const logger = pino({ level: "silent" });

const [, , target, textFile] = process.argv;
if (!target || !textFile) {
  console.error("Usage: node send-text.mjs <jid-or-number> <textfile>");
  process.exit(1);
}
const jid = target.includes("@") ? target : `${target.replace(/\D/g, "")}@s.whatsapp.net`;
const text = readFileSync(textFile, "utf8");

async function start() {
  const { state, saveCreds } = await useMultiFileAuthState(AUTH_DIR);
  const { version } = await fetchLatestBaileysVersion();
  const sock = makeWASocket({
    version,
    logger,
    auth: { creds: state.creds, keys: makeCacheableSignalKeyStore(state.keys, logger) },
    browser: ["eudamed2firstbase-sync", "CLI", "1.0"],
    markOnlineOnConnect: false,
  });
  sock.ev.on("creds.update", saveCreds);

  let done = false;
  sock.ev.on("connection.update", async (u) => {
    const { connection, lastDisconnect } = u;
    if (connection === "open") {
      try {
        const res = await sock.sendMessage(jid, { text });
        console.error(`[sent] to ${jid} id=${res?.key?.id}`);
        done = true;
        // delay so Baileys flushes creds + delivery
        setTimeout(() => { sock.end(); process.exit(0); }, 6000);
      } catch (e) {
        console.error("[send failed]", e?.message || e);
        process.exit(1);
      }
    }
    if (connection === "close") {
      const code = lastDisconnect?.error?.output?.statusCode;
      if (!done && code === 515) return setTimeout(() => start().catch(() => process.exit(1)), 1500);
      if (!done) { console.error(`[close] code=${code}`); process.exit(1); }
    }
  });
}
start().catch((e) => { console.error("fatal:", e?.message || e); process.exit(1); });
