import React from "react";
import { ChevronLeft, ChevronRight, ChevronsLeft, ChevronsRight } from "lucide-react";

export interface PaginationProps {
  page: number;
  pageSize: number;
  total: number;
  onPageChange: (page: number) => void;
  onPageSizeChange?: (size: number) => void;
  pageSizeOptions?: number[];
  labels?: { total?: string; perPage?: string };
}

/**
 * Pagination — NewAPI-style footer: 总计 N · 每页 N 条 · 首页/上一页/下一页/末页.
 * Fully custom buttons, no native select (page-size uses ui-select via parent
 * or built-in pill buttons).
 */
export function Pagination({
  page,
  pageSize,
  total,
  onPageChange,
  onPageSizeChange,
  pageSizeOptions = [10, 20, 50, 100],
  labels,
}: PaginationProps) {
  const totalPages = Math.max(1, Math.ceil(total / pageSize));
  const canPrev = page > 1;
  const canNext = page < totalPages;
  const t = labels ?? {};
  return (
    <div className="ui-pagination">
      <span className="ui-pagination-total">
        {t.total ?? "Total"}: {total}
      </span>
      {onPageSizeChange && (
        <div className="ui-pagination-size">
          <span>{t.perPage ?? "Per page"}</span>
          {pageSizeOptions.map((size) => (
            <button
              key={size}
              type="button"
              className={`ui-pagination-size-btn${size === pageSize ? " is-active" : ""}`}
              onClick={() => onPageSizeChange(size)}
            >
              {size}
            </button>
          ))}
        </div>
      )}
      <div className="ui-pagination-nav">
        <button type="button" disabled={!canPrev} onClick={() => onPageChange(1)} aria-label="First page">
          <ChevronsLeft size={14} />
        </button>
        <button type="button" disabled={!canPrev} onClick={() => onPageChange(page - 1)} aria-label="Previous page">
          <ChevronLeft size={14} />
        </button>
        <span className="ui-pagination-page">
          {page} / {totalPages}
        </span>
        <button type="button" disabled={!canNext} onClick={() => onPageChange(page + 1)} aria-label="Next page">
          <ChevronRight size={14} />
        </button>
        <button type="button" disabled={!canNext} onClick={() => onPageChange(totalPages)} aria-label="Last page">
          <ChevronsRight size={14} />
        </button>
      </div>
    </div>
  );
}

export default Pagination;
