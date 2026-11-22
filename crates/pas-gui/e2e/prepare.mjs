import { execFileSync } from "node:child_process";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const guiDir = path.resolve(__dirname, "..");
const repoRoot = path.resolve(__dirname, "../../..");
const targetDir = path.join(repoRoot, "target/e2e");
const cargoTauriBin =
  process.env.CARGO_TAURI_BIN || path.join(os.homedir(), ".cargo/bin/cargo-tauri");

const env = {
  ...process.env,
  CARGO_TARGET_DIR: targetDir,
};

execFileSync("cargo", ["build", "-p", "pasd", "-p", "pas-cli", "-p", "fake-agent"], {
  cwd: repoRoot,
  env,
  stdio: "inherit",
});

execFileSync(cargoTauriBin, ["build", "--debug", "--no-bundle"], {
  cwd: guiDir,
  env,
  stdio: "inherit",
});
