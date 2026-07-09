import { useStore } from "../store/wsStore";

export default function Footer() {
  const model = useStore((s) => s.llm_model);

  const text = `model: ${model || "—"}`;

  return <footer className="footer">{text}</footer>;
}
