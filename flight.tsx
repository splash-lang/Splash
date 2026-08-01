// ledger flight@1.0.0 · level L0 · profile ui/l0

export const current = source.news({
  count: 1,
  fields: ["id", "number", "from", "to", "sched_dep", "act_dep", "status"],
});
export const upcoming = source.news({ count: 7, offset: 1, fields: ["id", "number", "from", "to"] });

export const selected = state.text("");

export const select_flight = event({ selected: set.payload });
export const back = event({ selected: clear });

export const copy = {
  flight: { class: vocabulary, en: "FLIGHT", zh: "航班" },
  departure: { class: vocabulary, en: "DEPARTURE", zh: "出发" },
};

export function FlightRow({ flight, position, onOpen }: Props<{
  flight: record;
  position: number;
  onOpen: event;
}>) {
  return (
    <Col>
      <Row align=".center" onTap={onOpen} value={flight.id}>
        <TextRow text={position} width=".rank" />
        <TextRow text={flight.number} width=".fill" />
        <TextCaption text={flight.status} />
      </Row>
      <Rule />
    </Col>
  );
}

export function Root() {
  return (
    <Surface pad=".page">
      <When path={selected} eq="">
        <Panel>
          <For each={upcoming} key="id" as="f" index="i">
            <FlightRow flight={f} position={i} onOpen={select_flight} />
          </For>
        </Panel>
      </When>
      <When path={selected} neq="">
        <Col>
          <TextCaption text={copy.flight} />
          <TextHero text={current.number} />
        </Col>
      </When>
    </Surface>
  );
}
