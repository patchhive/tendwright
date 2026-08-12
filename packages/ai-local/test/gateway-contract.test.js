import assert from "node:assert/strict";
import test from "node:test";

import { copilotCredentialMode } from "../src/auth-contract.js";

test("maps authenticated Copilot sessions to the adapter credential enum", () => {
  assert.equal(copilotCredentialMode(true), "access_token");
  assert.equal(copilotCredentialMode(false), null);
});
