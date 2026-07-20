// Strongly-typed ID strings, mirrors of `bk_core::ids::Id<T>`
// (see `crates/bk-core/src/ids.rs`).
//
// Rust-side `Id<T>` is a zero-sized newtype around `Uuid`; the
// serde repr is a plain string. The phantom type parameter
// doesn't survive serialization, so the wire shape is just
// `string` — TypeScript doesn't have phantom types, so we use
// distinct branded string types to keep the same compile-time
// separation. The brand is the suffix on the type name; the
// wire representation is identical to a plain string.
//
// The opaque `__brand` field prevents accidental cross-typing:
// `exchangeId as ProjectId` is a TS error.
//
// Usage:
//   const id: ProjectId = "00000000-0000-0000-0000-000000000000";
//   function openProject(id: ProjectId) { ... }
//   openProject(exchangeId); // type error

declare const __brand: unique symbol;
type Brand<T, B> = T & { readonly [__brand]: B };

export type ProjectId = Brand<string, "Project">;
export type ExchangeId = Brand<string, "Exchange">;
export type TagId = Brand<string, "Tag">;
export type NoteId = Brand<string, "Note">;
export type FuzzJobId = Brand<string, "FuzzJob">;

/** Cast a plain string to a branded id. Use sparingly. */
export const asProjectId = (s: string): ProjectId => s as ProjectId;
export const asExchangeId = (s: string): ExchangeId => s as ExchangeId;
export const asTagId = (s: string): TagId => s as TagId;
export const asNoteId = (s: string): NoteId => s as NoteId;
export const asFuzzJobId = (s: string): FuzzJobId => s as FuzzJobId;
