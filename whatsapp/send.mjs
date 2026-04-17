#!/usr/bin/env node
// Send a file (image or document) to a WhatsApp chat via Baileys.
// Usage: node send.mjs <jid> <file-path> [caption]
// First run: scan QR code with WhatsApp → session saved in auth/

import makeWASocket, {
  useMultiFileAuthState,
  makeCacheableSignalKeyStore,
  fetchLatestBaileysVersion,
  DisconnectReason,
} from "@whiskeysockets/baileys";
import qrcode from "qrcode-terminal";
import pino from "pino";
import { readFileSync, existsSync, rmSync } from "fs";
import { resolve, dirname, basename, extname } from "path";
import { fileURLToPath } from "url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const AUTH_DIR = resolve(__dirname, "auth");
const logger = pino({ level: "silent" });

const [,, rawJid, filePath, ...captionParts] = process.argv;
const caption = captionParts.join(" ") || "";

if (!rawJid || !filePath) {
  console.error("Usage: node send.mjs <jid> <file-path> [caption]");
  console.error("  jid: group JID (120...@g.us) or phone (4179...@s.whatsapp.net)");
  process.exit(1);
}

const absPath = resolve(filePath);
if (!existsSync(absPath)) {
  console.error(`File not found: ${absPath}`);
  process.exit(1);
}

const jid = rawJid.includes("@")
  ? rawJid
  : (rawJid.length > 15 ? `${rawJid}@g.us` : `${rawJid}@s.whatsapp.net`);

const ext = extname(absPath).toLowerCase();
const fileName = basename(absPath);
const MIME = {
  ".png":  "image/png",
  ".jpg":  "image/jpeg",
  ".jpeg": "image/jpeg",
  ".pdf":  "application/pdf",
  ".html": "text/html",
  ".htm":  "text/html",
  ".json": "application/json",
  ".csv":  "text/csv",
  ".xlsx": "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
  ".xml":  "application/xml",
  ".txt":  "text/plain",
};
const mimetype = MIME[ext] || "application/octet-stream";
const isImage = mimetype.startsWith("image/");

let retries = 0;
const MAX_RETRIES = 3;
let done = false;

async function connect() {
  const { state, saveCreds } = await useMultiFileAuthState(AUTH_DIR);
  const { version } = await fetchLatestBaileysVersion();

  console.log(`Using WA version: ${version.join(".")}`);

  const sock = makeWASocket({
    auth: {
      creds: state.creds,
      keys: makeCacheableSignalKeyStore(state.keys, logger),
    },
    version,
    logger,
    browser: ["eudamed2firstbase", "CLI", "1.0"],
    syncFullHistory: false,
    markOnlineOnConnect: false,
  });

  sock.ev.on("creds.update", saveCreds);

  return new Promise((resolvePromise, reject) => {
    const timeout = setTimeout(() => {
      sock.end();
      reject(new Error("Connection timeout (90s)"));
    }, 90000);

    sock.ev.on("connection.update", async (update) => {
      const { connection, lastDisconnect, qr } = update;

      if (qr) {
        // Sentinel line for the Rust parent to render natively (GUI).
        process.stdout.write(`__QR__:${qr}\n`);
        console.log("\n  Scan this QR code with WhatsApp:");
        console.log("  (Settings > Linked Devices > Link a Device)\n");
        qrcode.generate(qr, { small: true });
        console.log("  Waiting for scan...\n");
      }

      if (connection === "open") {
        clearTimeout(timeout);
        try {
          console.log(`Connected! Sending ${isImage ? "image" : "document"} ${fileName} to ${jid}...`);

          const buffer = readFileSync(absPath);
          const message = isImage
            ? { image: buffer, caption, mimetype }
            : { document: buffer, mimetype, fileName, caption };

          const sendResult = await Promise.race([
            sock.sendMessage(jid, message),
            new Promise((_, rej) => setTimeout(() => rej(new Error("sendMessage timeout (60s)")), 60000)),
          ]);

          console.log("Sent!", sendResult?.key?.id ? `(id: ${sendResult.key.id})` : "");
          done = true;
          setTimeout(() => process.exit(0), 1000);
        } catch (err) {
          console.error("Send error:", err.message);
          sock.end();
          reject(err);
        }
      }

      if (connection === "close") {
        clearTimeout(timeout);
        if (done) {
          resolvePromise();
          return;
        }
        const statusCode = lastDisconnect?.error?.output?.statusCode;

        if (statusCode === DisconnectReason.loggedOut) {
          console.log("Session expired. Clearing auth, please scan again...");
          rmSync(AUTH_DIR, { recursive: true, force: true });
          retries = 0;
          connect().then(resolvePromise).catch(reject);
        } else if (retries < MAX_RETRIES) {
          retries++;
          connect().then(resolvePromise).catch(reject);
        } else {
          reject(new Error(`Connection failed after ${MAX_RETRIES} retries (status: ${statusCode})`));
        }
      }
    });
  });
}

connect()
  .then(() => process.exit(0))
  .catch((err) => {
    console.error("Error:", err.message);
    process.exit(1);
  });
