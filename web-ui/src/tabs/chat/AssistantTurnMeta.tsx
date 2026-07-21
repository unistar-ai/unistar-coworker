export default function AssistantTurnMeta({ text }: { text: string }) {
  return (
    <span className="assistant-turn-meta" title={text}>
      {text}
    </span>
  );
}
