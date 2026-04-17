#!/usr/bin/env node
// List all WhatsApp groups with their JIDs.
// Usage: node list-groups.mjs
// Requires: session already set up (run send.mjs first to scan QR).

import makeWASocket, {
  useMultiFileAuthState,
  makeCacheableSignalKeyStore,
  fetchLatestBaileysVersion,
} from "@whiskeysockets/baileys";
import qrcode from "qrcode-terminal";
import pino from "pino";
import { resolve, dirname } from "path";
import { fileURLToPath } from "url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const AUTH_DIR = resolve(__dirname, "auth");
const logger = pino({ level: "silent" });

let done = false;

async function main() {
  const { state, saveCreds } = await useMultiFileAuthState(AUTH_DIR);
  const { version } = await fetchLatestBaileysVersion();

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

  await new Promise((resolvePromise, reject) => {
    const timeout = setTimeout(() => {
      sock.end();
      if (!done) reject(new Error("Connection timeout (90s)"));
    }, 90000);

    sock.ev.on("connection.update", async (update) => {
      const { connection, qr } = update;

      if (qr) {
        process.stdout.write(`__QR__:${qr}\n`);
        console.log("\n  Scan this QR code with WhatsApp:");
        console.log("  (Settings > Linked Devices > Link a Device)\n");
        qrcode.generate(qr, { small: true });
      }

      if (connection === "open") {
        clearTimeout(timeout);
        try {
          console.log("Connected. Fetching groups...\n");
          const groups = await sock.groupFetchAllParticipating();
          const sorted = Object.values(groups).sort((a, b) =>
            a.subject.localeCompare(b.subject)
          );
          console.log(`Found ${sorted.length} groups:\n`);
          for (const g of sorted) {
            console.log(`  ${g.id}  ${g.subject}`);
          }
          done = true;
          // Force exit — sock.end() triggers a close handler that races us.
          setTimeout(() => process.exit(0), 500);
          resolvePromise();
        } catch (err) {
          reject(err);
        }
      }

      if (connection === "close") {
        clearTimeout(timeout);
        // Don't reject if we already fetched the groups successfully.
        if (done) {
          resolvePromise();
        } else {
          reject(new Error("Connection closed before pairing completed"));
        }
      }
    });
  });
}

main()
  .then(() => process.exit(0))
  .catch((err) => {
    if (done) process.exit(0);
    console.error("Error:", err.message);
    process.exit(1);
  });
