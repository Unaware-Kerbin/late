import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { createRpcQueue } from "./mcp-rpc-queue.ts";

describe("mcp rpc queue", () => {
  it("runs a later grant after an in-flight chat_get, without aborting it", async () => {
    const enqueue = createRpcQueue();
    const order: string[] = [];
    let chatFinished = false;
    const chat = enqueue(async () => {
      order.push("chat-start");
      await new Promise((r) => setTimeout(r, 30));
      chatFinished = true;
      order.push("chat-end");
      return "chat_get";
    });
    const grant = enqueue(async () => {
      assert.equal(chatFinished, true);
      order.push("grant");
      return "add_allowed_dir";
    });
    const [chatOut, grantOut] = await Promise.all([chat, grant]);
    assert.equal(chatOut, "chat_get");
    assert.equal(grantOut, "add_allowed_dir");
    assert.deepEqual(order, ["chat-start", "chat-end", "grant"]);
  });

  it("a rejected grant does not reject the in-flight chat rpc", async () => {
    const enqueue = createRpcQueue();
    const chat = enqueue(async () => {
      await new Promise((r) => setTimeout(r, 20));
      return "ok";
    });
    const grant = enqueue(async () => {
      throw new Error("ENOENT: no such file or directory");
    });
    assert.equal(await chat, "ok");
    await assert.rejects(grant, /ENOENT/);
  });
});
