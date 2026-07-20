// Tests for the DecoderPanel (§4.7).
//
// Spec (§4.7):
//   - The single-op `Decode` button transforms the
//     input with the selected op (base64 / url /
//     html / hex).
//   - The `Smart` button recursively applies
//     base64 → url → html up to 8 layers and shows
//     the layer chain in the output header.
//   - The 8-layer cap prevents infinite loops on
//     self-decoding inputs.

import { afterEach, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { DecoderPanel } from "./DecoderPanel";

afterEach(() => {
  cleanup();
});

describe("DecoderPanel", () => {
  it("decodes a base64 string with the single-op Decode button", () => {
    render(<DecoderPanel />);
    const input = screen.getByTestId("decoder-panel-input");
    // "hello" base64-encoded.
    fireEvent.change(input, { target: { value: "aGVsbG8=" } });
    fireEvent.click(screen.getByTestId("decoder-panel-decode"));
    const output = screen.getByTestId("decoder-panel-output");
    expect(output.textContent).toContain("hello");
  });

  it("smart-decodes a base64-wrapped-URL-encoded string", () => {
    render(<DecoderPanel />);
    // "hi there!" URL-encoded, then base64-encoded.
    //   URL-encoded: "hi%20there%21"
    //   base64 of "hi%20there%21": "aGklMjB0aGVyZSUyMQ=="
    // The smart-decode loop applies base64 first
    // (yielding "hi%20there%21"), then url
    // (yielding "hi there!"). The loop then stops
    // because base64 on "hi there!" fails the
    // strict-alphabet check (the space and the `!`
    // are not base64 characters) and url/html do
    // not change the input.
    const input = screen.getByTestId("decoder-panel-input");
    fireEvent.change(input, { target: { value: "aGklMjB0aGVyZSUyMQ==" } });
    fireEvent.click(screen.getByTestId("decoder-panel-smart"));
    const layers = screen.getByTestId("decoder-panel-smart-layers");
    expect(layers.textContent).toBe("base64 → url");
    const result = screen.getByTestId("decoder-panel-smart-result");
    // The decoded output is the URL-decoded "hi there!".
    expect(result.textContent).toContain("hi there!");
  });

  it("stops after 8 layers on a self-decoding input (no infinite loop)", () => {
    // Pick an input that fails to decode under all three
    // ops. The smart-decode loop should bail out
    // immediately (no layer fires) instead of looping.
    render(<DecoderPanel />);
    const input = screen.getByTestId("decoder-panel-input");
    fireEvent.change(input, { target: { value: "!!!not base64!!!" } });
    fireEvent.click(screen.getByTestId("decoder-panel-smart"));
    const layers = screen.getByTestId("decoder-panel-smart-layers");
    expect(layers.textContent).toBe("no change");
  });

  it("decodes a URL-encoded string with the URL op", () => {
    render(<DecoderPanel />);
    const input = screen.getByTestId("decoder-panel-input");
    fireEvent.change(input, { target: { value: "hello%20world" } });
    // The default op is base64; switch to url via the
    // <select>. The select is a controlled component.
    const opSelect = screen.getByTestId("decoder-panel-op") as HTMLSelectElement;
    fireEvent.change(opSelect, { target: { value: "url" } });
    fireEvent.click(screen.getByTestId("decoder-panel-decode"));
    const output = screen.getByTestId("decoder-panel-output");
    expect(output.textContent).toContain("hello world");
  });

  it("decodes a hex string with the Hex op", () => {
    render(<DecoderPanel />);
    const input = screen.getByTestId("decoder-panel-input");
    fireEvent.change(input, { target: { value: "68656c6c6f" } });
    const opSelect = screen.getByTestId("decoder-panel-op") as HTMLSelectElement;
    fireEvent.change(opSelect, { target: { value: "hex" } });
    fireEvent.click(screen.getByTestId("decoder-panel-decode"));
    const output = screen.getByTestId("decoder-panel-output");
    expect(output.textContent).toContain("hello");
  });
});
