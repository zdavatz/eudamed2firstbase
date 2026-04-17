#!/usr/bin/env node
// List WhatsApp 1:1 contacts (and group chats) by JID.
// Usage: node list-contacts.mjs [search-term]
// Caveat: Baileys only sees contacts/chats it has been told about by the
// phone — you'll get whoever has messaged you while this session was alive
// plus anyone the phone has synced via "messaging-history.set".

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

const search = (process.argv[2] || "").toLowerCase();

let done = false;
const contacts = new Map(); // jid -> { id, name, notify }

function record(c) {
  if (!c?.id) return;
  // Keep the best name we've seen.
  const existing = contacts.get(c.id);
  const merged = {
    id: c.id,
    name: c.name || c.notify || c.verifiedName || existing?.name || "",
    notify: c.notify || existing?.notify || "",
  };
  contacts.set(c.id, merged);
}

function dump() {
  const rows = [...contacts.values()]
    .filter((c) => {
      if (!search) return true;
      const hay = `${c.id} ${c.name} ${c.notify}`.toLowerCase();
      return hay.includes(search);
    })
    .sort((a, b) => (a.name || a.id).localeCompare(b.name || b.id));
  console.log(`\nFound ${rows.length} entries${search ? ` matching "${search}"` : ""}:\n`);
  for (const c of rows) {
    const label = c.name || c.notify || "(unknown)";
    console.log(`  ${c.id.padEnd(40)} ${label}`);
  }
}

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
    syncFullHistory: true,
    markOnlineOnConnect: false,
  });

  sock.ev.on("creds.update", saveCreds);

  // Contacts pushed by Baileys at any time.
  sock.ev.on("contacts.upsert", (cs) => cs.forEach(record));
  sock.ev.on("contacts.update", (cs) => cs.forEach(record));
  sock.ev.on("chats.upsert", (cs) => cs.forEach(record));

  // Initial bulk sync of chats/contacts/messages from the phone.
  sock.ev.on("messaging-history.set", ({ chats, contacts: ctx }) => {
    chats?.forEach(record);
    ctx?.forEach(record);
  });

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
        console.log("Connected. Waiting up to 12s for contact/chat sync…");
        // Give the phone a moment to push messaging-history.set.
        setTimeout(() => {
          clearTimeout(timeout);
          // Also include groups via the explicit fetch (more complete than chats.upsert alone).
          sock
            .groupFetchAllParticipating()
            .then((groups) => {
              for (const g of Object.values(groups)) {
                record({ id: g.id, name: g.subject });
              }
            })
            .catch(() => {})
            .finally(() => {
              dump();
              done = true;
              setTimeout(() => process.exit(0), 300);
              resolvePromise();
            });
        }, 12000);
      }

      if (connection === "close") {
        clearTimeout(timeout);
        if (done) {
          resolvePromise();
        } else {
          reject(new Error("Connection closed before sync completed"));
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
