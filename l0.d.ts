// Ambient declarations for L0-as-TSX.
//
// These exist so an editor can type-check a card without any of it being real:
// nothing here is executed, and the shapes are the ones §2 declares. If this
// file and the canonical grammar ever disagree, the grammar wins.

declare type record = { [k: string]: unknown };
declare type collection = record[];
declare type event = { readonly __event: unique symbol };
declare type text = string;

declare type Props<T> = T;

// ── declarations ─────────────────────────────────────────────────────────────
declare const source: {
  news(a: { count?: number; offset?: number; fields?: string[] }): collection;
  news_item(a: { id: unknown; fields?: string[] }): record;
  weather(a: { lat: unknown; lon: unknown; days?: number; fields?: string[] }): record;
  movers(a: { count?: number; fields?: string[] }): collection;
  quote(a: { ticker: unknown; fields?: string[] }): record;
  geocode(a: { name: unknown }): record;
  locale(): record;
};

declare const state: {
  text(initial: string): text;
  number(initial: number): number;
  bool(initial: boolean): boolean;
  enumOf<T extends string>(members: readonly T[], initial: T): T;
};

declare function event(transitions: Record<string, unknown>): event;
declare const set: { payload: unknown; to(v: unknown): unknown };
declare const clear: unknown;
declare const toggle: unknown;
declare const vocabulary: unique symbol;
declare const user_copy: unique symbol;
declare const model_copy: unique symbol;

// ── constructors (the closed catalog) ────────────────────────────────────────
//
// Declared as VALUES, not JSX.IntrinsicElements. A capitalised tag in JSX is a
// variable reference — `IntrinsicElements` only ever covers lowercase tags — so
// `<Col>` looks for a binding named `Col`. Keeping L0's capitalisation means
// declaring each constructor, which is also how React treats a component.

declare type Node = { readonly __node: unique symbol };
declare type Ctor<P> = (props: P) => Node;

declare namespace JSX {
  type Element = Node;
  interface ElementChildrenAttribute { children: {} }
}

type Kids = { children?: unknown };

declare const Surface: Ctor<{ pad?: string } & Kids>;
declare const Panel: Ctor<Kids>;
declare const Card: Ctor<{ onTap?: event; value?: unknown } & Kids>;
declare const Col: Ctor<{ gap?: number; align?: string } & Kids>;
declare const Row: Ctor<{ gap?: number; align?: string; onTap?: event; value?: unknown } & Kids>;
declare const Grid: Ctor<{ cols?: number } & Kids>;
declare const Rule: Ctor<{}>;
declare const TextHero: Ctor<{ text?: unknown; value?: unknown; unit?: string; format?: string }>;
declare const TextTitle: Ctor<{ text?: unknown; width?: string }>;
declare const TextBody: Ctor<{ text?: unknown; width?: string }>;
declare const TextRow: Ctor<{ text?: unknown; width?: string }>;
declare const TextCaption: Ctor<{ text?: unknown; value?: unknown; glyph?: string; suffix?: unknown; width?: string }>;
declare const TextValue: Ctor<{ value?: unknown; unit?: string; format?: string; tint?: unknown }>;
declare const TextStat: Ctor<{ value?: unknown; format?: string; tint?: unknown }>;
declare const Tile: Ctor<{ label?: unknown; value?: unknown; unit?: string; format?: string }>;
declare const Chip: Ctor<{ text?: unknown; onTap?: event; value?: unknown; active?: boolean }>;

// structural forms
declare const For: Ctor<{ each: unknown; key: string; as: string; index?: string } & Kids>;
declare const When: Ctor<{ path: unknown; eq?: string; neq?: string } & Kids>;
declare const Slot: Ctor<{ name?: string }>;
