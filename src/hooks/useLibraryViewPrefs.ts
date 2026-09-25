import { useEffect, useState } from "react";
import * as api from "../lib/tauri";
import {
  LIBRARY_GROUP_BY_OPTIONS,
  LIBRARY_SORT_BY_OPTIONS,
  type LibraryGroupBy,
  type LibrarySortBy,
} from "../lib/librarySkillQuery";

const GROUP_BY_SETTING = "library_group_by";
const SORT_BY_SETTING = "library_sort_by";
const isGroupBy = (v: string | null): v is LibraryGroupBy =>
  LIBRARY_GROUP_BY_OPTIONS.includes(v as LibraryGroupBy);
const isSortBy = (v: string | null): v is LibrarySortBy =>
  LIBRARY_SORT_BY_OPTIONS.includes(v as LibrarySortBy);

/**
 * Group and sort for the library. They are layout preferences, so they are
 * loaded from settings and saved on every change; filters are per-session.
 */
export function useLibraryViewPrefs() {
  const [groupBy, setGroupBy] = useState<LibraryGroupBy>("none");
  const [sortBy, setSortBy] = useState<LibrarySortBy>("name");

  useEffect(() => {
    Promise.all([api.getSettings(GROUP_BY_SETTING), api.getSettings(SORT_BY_SETTING)])
      .then(([savedGroup, savedSort]) => {
        if (isGroupBy(savedGroup)) setGroupBy(savedGroup);
        if (isSortBy(savedSort)) setSortBy(savedSort);
      })
      .catch(() => {
        // defaults are fine
      });
  }, []);

  const chooseGroupBy = (value: LibraryGroupBy) => {
    setGroupBy(value);
    void api.setSettings(GROUP_BY_SETTING, value).catch(() => {});
  };
  const chooseSortBy = (value: LibrarySortBy) => {
    setSortBy(value);
    void api.setSettings(SORT_BY_SETTING, value).catch(() => {});
  };

  return { groupBy, sortBy, chooseGroupBy, chooseSortBy };
}
