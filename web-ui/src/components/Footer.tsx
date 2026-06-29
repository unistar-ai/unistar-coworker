import { useStore } from "../store/wsStore";

export default function Footer() {
  const attachMode = useStore((s) => s.attach_mode);
  const model = useStore((s) => s.llm_model);
  const repos = useStore((s) => s.repos);

  const text = attachMode
    ? "attach mode · shared store with daemon"
    : `model: ${model || "—"} · repos: ${(repos || []).join(", ") || "—"}`;

  return <footer className="footer">{text}</footer>;
}
