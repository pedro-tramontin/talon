// JSON tree-view for the `ReplayRequestEditor`'s "Pretty"
// sub-tab (Phase 7 C-B.5).
//
// A recursive component that renders a JSON value (object,
// array, primitive) as nested lists. No `react-json-view`
// dep — the tree is small (cap depth at 10 levels
// defensively per the `verify:` block's "no infinite
// recursion" requirement) and the styling follows the
// rest of the v0.5 editor's Tailwind classes.
//
// ## Cap on depth
//
// The render is recursive: an object renders its keys, each
// key's value renders recursively. A circular or
// pathologically-deep JSON value would blow the stack.
// We cap the depth at 10 — beyond that, the value renders
// as a `[depth-capped]` placeholder (so the user sees that
// the data exists but the tree stops there).
//
// ## Collapse / expand
//
// A `useState<Set<string>>` of *collapsed* paths. The
// path is the dot-joined key sequence (e.g. `"a.b.c"`
// for an object `{ a: { b: { c: ... } } }`). Array
// indices use numeric keys (e.g. `"a.0.b"`). Default:
// all paths expanded. The `▶` / `▼` toggle on each
// object/array node adds/removes the path from the set.
//
// ## data-testid
//
// The root has `data-testid="json-tree-view"`; child nodes
// use the path as a suffix (e.g. `data-testid="json-tree-
// view-node-a.b.c"`). This is enough for the
// `ReplayRequestEditor` tests to assert the tree rendered.

import { useState } from "react";

interface JsonTreeViewProps {
  /** The JSON value to render. Must be a JSON-serializable
   * value (object, array, string, number, boolean, null). */
  value: unknown;
  /** The dot-joined key path leading to this node. The
   * root node has an empty path. */
  path?: string;
  /** Current render depth (root = 0). Used to enforce the
   * depth cap. The component increments this on each
   * recursive call; the parent should NOT pass a value
   * here (it defaults to 0). */
  depth?: number;
}

const MAX_DEPTH = 10;

export function JsonTreeView({
  value,
  path = "",
  depth = 0,
}: JsonTreeViewProps) {
  // Set of paths the user has *collapsed* (default:
  // all paths expanded). The toggle adds/removes a path
  // from this set. Per-component state is OK here
  // because the user toggles a node, and that node's
  // state is local — children re-render through React's
  // normal tree update.
  const [collapsed, setCollapsed] = useState<Set<string>>(
    () => new Set(),
  );

  if (depth >= MAX_DEPTH) {
    return (
      <div
        data-testid={`json-tree-view-depth-capped${path ? `-${path}` : ""}`}
        className="ml-4 text-xs italic text-slate-500"
      >
        [depth-capped]
      </div>
    );
  }

  // Toggle a path's collapse state. If the path is in
  // the set, remove it (expand); if not, add it
  // (collapse). The root path "" is not toggleable
  // (we always show the top-level value).
  const toggle = (p: string) => {
    if (p === "") return;
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(p)) {
        next.delete(p);
      } else {
        next.add(p);
      }
      return next;
    });
  };

  // Primitives: render inline.
  if (value === null) {
    return (
      <span
        data-testid={`json-tree-view-null${path ? `-${path}` : ""}`}
        className="font-mono text-xs text-slate-500"
      >
        null
      </span>
    );
  }
  if (typeof value === "string") {
    return (
      <span
        data-testid={`json-tree-view-string${path ? `-${path}` : ""}`}
        className="font-mono text-xs text-green-300"
      >
        "{value}"
      </span>
    );
  }
  if (typeof value === "number") {
    return (
      <span
        data-testid={`json-tree-view-number${path ? `-${path}` : ""}`}
        className="font-mono text-xs text-cyan-300"
      >
        {String(value)}
      </span>
    );
  }
  if (typeof value === "boolean") {
    return (
      <span
        data-testid={`json-tree-view-boolean${path ? `-${path}` : ""}`}
        className="font-mono text-xs text-yellow-300"
      >
        {String(value)}
      </span>
    );
  }

  // Objects: a list of <key>: <value> pairs.
  if (typeof value === "object" && !Array.isArray(value)) {
    const entries = Object.entries(value as Record<string, unknown>);
    if (entries.length === 0) {
      return (
        <span
          data-testid={`json-tree-view-empty-object${path ? `-${path}` : ""}`}
          className="font-mono text-xs text-slate-500"
        >
          {"{}"}
        </span>
      );
    }
    const isCollapsed = path !== "" && collapsed.has(path);
    return (
      <div
        data-testid={`json-tree-view-object${path ? `-${path}` : ""}`}
        className="font-mono text-xs"
      >
        {path !== "" && (
          <button
            type="button"
            data-testid={`json-tree-view-toggle-${path}`}
            onClick={() => toggle(path)}
            className="mr-1 text-slate-400 hover:text-slate-200"
          >
            {isCollapsed ? "▶" : "▼"}
          </button>
        )}
        <span className="text-slate-400">{"{"}</span>
        {isCollapsed ? (
          <span className="text-slate-500"> ... </span>
        ) : (
          <ul className="ml-4 list-none">
            {entries.map(([k, v]) => {
              const childPath = path ? `${path}.${k}` : k;
              return (
                <li key={k} className="py-0.5">
                  <span
                    data-testid={`json-tree-view-key-${childPath}`}
                    className="text-slate-300"
                  >
                    "{k}"
                  </span>
                  <span className="text-slate-500">: </span>
                  <JsonTreeView
                    value={v}
                    path={childPath}
                    depth={depth + 1}
                  />
                  <span className="text-slate-500">,</span>
                </li>
              );
            })}
          </ul>
        )}
        <span className="text-slate-400">{"}"}</span>
      </div>
    );
  }

  // Arrays: a list of <value> items with numeric keys.
  if (Array.isArray(value)) {
    if (value.length === 0) {
      return (
        <span
          data-testid={`json-tree-view-empty-array${path ? `-${path}` : ""}`}
          className="font-mono text-xs text-slate-500"
        >
          {"[]"}
        </span>
      );
    }
    const isCollapsed = path !== "" && collapsed.has(path);
    return (
      <div
        data-testid={`json-tree-view-array${path ? `-${path}` : ""}`}
        className="font-mono text-xs"
      >
        {path !== "" && (
          <button
            type="button"
            data-testid={`json-tree-view-toggle-${path}`}
            onClick={() => toggle(path)}
            className="mr-1 text-slate-400 hover:text-slate-200"
          >
            {isCollapsed ? "▶" : "▼"}
          </button>
        )}
        <span className="text-slate-400">[</span>
        {isCollapsed ? (
          <span className="text-slate-500"> ... </span>
        ) : (
          <ul className="ml-4 list-none">
            {value.map((v, i) => {
              const childPath = path ? `${path}.${i}` : String(i);
              return (
                <li key={i} className="py-0.5">
                  <JsonTreeView
                    value={v}
                    path={childPath}
                    depth={depth + 1}
                  />
                  <span className="text-slate-500">,</span>
                </li>
              );
            })}
          </ul>
        )}
        <span className="text-slate-400">]</span>
      </div>
    );
  }

  // Fallback: unknown value type (e.g. `undefined`, bigint,
  // symbol). We render as text — the editor's data flow
  // shouldn't emit these (the input is JSON), but the
  // fallback prevents a crash.
  return (
    <span
      data-testid={`json-tree-view-unknown${path ? `-${path}` : ""}`}
      className="font-mono text-xs text-slate-500"
    >
      {String(value)}
    </span>
  );
}
