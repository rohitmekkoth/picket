import { useEffect, useState } from "react";

// A small hash router. Notification click-through opens `#/incident/<id>` directly.

export type Route =
  | { page: "overview" }
  | { page: "review" }
  | { page: "incident"; id: string }
  | { page: "activity" }
  | { page: "rules" }
  | { page: "boundary"; id: string | null }
  | { page: "settings" };

export function parse(hash: string): Route {
  const path = hash.replace(/^#\/?/, "");
  const [page, ...rest] = path.split("/");
  switch (page) {
    case "review":
      return { page: "review" };
    case "incident":
      return rest[0] ? { page: "incident", id: `incident/${rest[0]}` } : { page: "review" };
    case "activity":
      return { page: "activity" };
    case "rules":
      if (rest[0] === "new") return { page: "boundary", id: null };
      if (rest[0] === "boundary" && rest[1]) return { page: "boundary", id: rest[1] };
      return { page: "rules" };
    case "settings":
      return { page: "settings" };
    default:
      return { page: "overview" };
  }
}

export function href(route: Route): string {
  switch (route.page) {
    case "overview":
      return "#/overview";
    case "review":
      return "#/review";
    case "incident":
      return `#/incident/${route.id.replace(/^incident\//, "")}`;
    case "activity":
      return "#/activity";
    case "rules":
      return "#/rules";
    case "boundary":
      return route.id ? `#/rules/boundary/${route.id}` : "#/rules/new";
    case "settings":
      return "#/settings";
  }
}

export function navigate(route: Route): void {
  window.location.hash = href(route);
}

export function useRoute(): Route {
  const [route, setRoute] = useState(() => parse(window.location.hash));
  useEffect(() => {
    const onChange = () => setRoute(parse(window.location.hash));
    window.addEventListener("hashchange", onChange);
    return () => window.removeEventListener("hashchange", onChange);
  }, []);
  return route;
}
