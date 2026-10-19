import React from "react";
import ErrorPage from "./ErrorPage";

/**
 * The view for a console path that does not exist.
 *
 * Before this the router's fallback silently redirected to the overview, so a
 * mistyped or stale link looked like a slow page rather than a wrong one.
 */
export default function NotFoundPage() {
  return <ErrorPage status={404} />;
}
