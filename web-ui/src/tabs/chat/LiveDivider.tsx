export default function LiveDivider({ visible }: { visible: boolean }) {
  if (!visible) return null;
  return (
    <div className="live-divider" aria-hidden="true">
      <span className="live-divider-line" />
      <span className="live-divider-text">In progress</span>
      <span className="live-divider-line" />
    </div>
  );
}
