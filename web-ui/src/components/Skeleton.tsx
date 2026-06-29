interface SkeletonProps {
  /** Number of shimmer rows to render. */
  rows?: number;
  /** Extra class for the wrapper. */
  className?: string;
}

/** Shimmer skeleton placeholder for loading states. Renders `rows` pulsing
 * bars of varying widths to mimic a list while data is being fetched. */
export default function Skeleton({ rows = 3, className = "" }: SkeletonProps) {
  return (
    <div className={`skeleton-list ${className}`} role="status" aria-label="Loading">
      {Array.from({ length: rows }).map((_, i) => (
        <div
          key={i}
          className={`skeleton-row ${i % 3 === 2 ? "short" : i % 3 === 1 ? "medium" : ""}`}
        />
      ))}
    </div>
  );
}
