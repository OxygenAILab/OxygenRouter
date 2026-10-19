import React from "react";
import { useQuery } from "@tanstack/react-query";
import { Link } from "react-router-dom";
import { api } from "../lib/api";
import { PageHeader } from "../App";
import { useI18n } from "../lib/i18nContext";

type Kind = "about" | "terms" | "privacy";

/**
 * The instance's own copy, rendered as it was written.
 *
 * Plain text rather than Markdown: the option is a text box in the console, and
 * rendering an operator's angle brackets as markup would be a way to inject
 * anything into the page from a settings field.
 */
export default function StaticPage({ kind }: { kind: Kind }) {
  const { t } = useI18n();
  const { data, isLoading } = useQuery({
    queryKey: ["legal", kind],
    queryFn: () =>
      kind === "about" ? api.legal.about() : kind === "terms" ? api.legal.terms() : api.legal.privacy(),
    staleTime: 60_000,
  });

  const title = kind === "about" ? t.static.about : kind === "terms" ? t.static.terms : t.static.privacy;
  const text = (data ?? "").trim();

  return (
    <div className="page-fade-enter space-y-4">
      <PageHeader
        title={title}
        description={t.static.subtitle}
        action={
          <Link className="btn-ghost btn-sm" to="/ui/overview">
            {t.static.back}
          </Link>
        }
      />
      <section className="dashboard-section">
        <div className="static-copy">
          {isLoading ? (
            <div className="skeleton h-40 w-full" />
          ) : text ? (
            text.split("\n").map((line, index) => (
              <p key={index} className={line.trim() === "" ? "is-blank" : ""}>
                {line}
              </p>
            ))
          ) : (
            <p className="pricing-muted">{t.static.empty}</p>
          )}
        </div>
      </section>
    </div>
  );
}
