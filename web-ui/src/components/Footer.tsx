import { useStore } from "../store/wsStore";

export default function Footer() {
  const model = useStore((s) => s.llm_model);
  const repos = useStore((s) => s.repos);

  const text = `model: ${model || "—"} · repos: ${(repos || []).join(", ") || "—"}`;

  return <footer className="footer">{text}</footer>;
}
