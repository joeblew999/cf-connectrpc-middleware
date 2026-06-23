import { describe, expect, it } from "vitest";
import { ConnectError, Code } from "@connectrpc/connect";
import { errorMessage } from "../src/client.js";

describe("errorMessage", () => {
  it("surfaces a ConnectError's message", () => {
    expect(errorMessage(new ConnectError("nope", Code.NotFound), "fallback")).toContain("nope");
  });
  it("returns the fallback for a non-ConnectError", () => {
    expect(errorMessage(new Error("boom"), "fallback")).toBe("fallback");
    expect(errorMessage("oops", "fallback")).toBe("fallback");
  });
});
