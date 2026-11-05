// The native notification probe from the menu opens this view, to check click-through.
export function NotificationTest({ action }: { action: "review" | "expected" }) {
  return (
    <div className="stack stack-narrow">
      <p className="eyebrow">Notification test</p>
      <h1>
        {action === "expected" ? "This was expected action received" : "Review action received"}
      </h1>
      <p className="lede" role="status">
        The native notification opened this window and selected this view.
      </p>
    </div>
  );
}
