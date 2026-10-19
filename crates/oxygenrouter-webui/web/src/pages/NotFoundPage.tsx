import React from "react";
import { Link } from "react-router-dom";
import { Compass } from "lucide-react";
import { useI18n } from "../lib/i18nContext";

/**
 * The view for a console path that does not exist.
 *
 * Before this the router's fallback silently redirected to the overview, so a
 * mistyped or stale link looked like a slow page rather than a wrong one.
 */
export default function NotFoundPage() {
  const { t } = useI18n();
  return (
    <div className="page-fade-enter not-found">
      <Compass size={28} />
      <h1>{t.notFound.title}</h1>
      <p className="pricing-muted">{t.notFound.body}</p>
      <Link className="btn-filled" to="/ui/overview">
        {t.notFound.back}
      </Link>
    </div>
  );
}
