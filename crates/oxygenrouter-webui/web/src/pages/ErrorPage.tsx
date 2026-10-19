import React from "react";
import { Link, useNavigate } from "react-router-dom";
import { Ban, Compass, ServerCrash, ShieldAlert, Wrench } from "lucide-react";
import { useI18n } from "../lib/i18nContext";

export type ErrorStatus = 401 | 403 | 404 | 500 | 503;

const ICONS = {
  401: ShieldAlert,
  403: Ban,
  404: Compass,
  500: ServerCrash,
  503: Wrench,
} as const;

/**
 * The console's own views for the status codes a gateway will produce.
 *
 * Keeping these in one component means a new status cannot drift into a
 * different layout or lose the recovery actions users need most.
 */
export default function ErrorPage({ status }: { status: ErrorStatus }) {
  const { t } = useI18n();
  const navigate = useNavigate();
  const Icon = ICONS[status];

  const copy = (() => {
    switch (status) {
      case 401:
        return { title: t.errors.unauthorizedTitle, body: t.errors.unauthorizedBody };
      case 403:
        return { title: t.errors.forbiddenTitle, body: t.errors.forbiddenBody };
      case 404:
        return { title: t.notFound.title, body: t.notFound.body };
      case 503:
        return { title: t.errors.maintenanceTitle, body: t.errors.maintenanceBody };
      default:
        return { title: t.errors.serverTitle, body: t.errors.serverBody };
    }
  })();

  return (
    <div className="page-fade-enter error-page" data-status={status}>
      <div className="error-page-code" aria-hidden="true">
        {status}
      </div>
      <div className="error-page-icon" aria-hidden="true">
        <Icon size={28} />
      </div>
      <h1>{copy.title}</h1>
      <p>{copy.body}</p>
      <div className="error-page-actions">
        <button className="btn-outlined" type="button" onClick={() => navigate(-1)}>
          {t.errors.goBack}
        </button>
        <Link className="btn-filled" to="/ui/overview">
          {t.errors.backHome}
        </Link>
      </div>
    </div>
  );
}
