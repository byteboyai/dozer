export type TodoState = 'pending' | 'in_progress' | 'suspended' | 'done';
export type SegmentKey = 'active' | 'paused' | 'done';
export type StatusFilter = 'all' | TodoState;

export interface TodoCard {
  id: number;
  text: string;
  state: TodoState;
  segment: SegmentKey;
  /** "MM-DD" 或 null */
  plan_date: string | null;
  /** 已完成时的完成日期("MM-DD"),否则 null */
  completed_label: string | null;
  category_id: number | null;
  /** null = 未分类 */
  category_name: string | null;
  /** AgentKind 的 snake_case 字符串,如 "claude" */
  assigned_agent: string | null;
  has_dispatch: boolean;
}

export interface CategoryRow {
  id: number;
  name: string;
  parent_id: number | null;
  depth: number;
}

export interface AgentInfo {
  kind: string;
  label: string;
  /** Rust 内嵌的 SVG 原文(可信来源) */
  icon_svg: string;
  preserves_color: boolean;
}

export interface TodayInfo {
  year: number;
  month: number;
  day: number;
}

export interface ViewPayload {
  project_id: number;
  /** all | uncategorized | node:<id> */
  category_key: string;
  items: TodoCard[];
  categories: CategoryRow[];
  agents: AgentInfo[];
  add_height_px: number;
  today: TodayInfo;
  /** Rust 在新增后高亮的卡片 id(沿用 Flash 计时),否则 null */
  selected_id: number | null;
  /** 每次 Rust 要求"滚回顶部"时递增 */
  scroll_nonce: number;
}

export type SetStatusTarget = 'pending' | 'suspended' | 'done';

export type OutEvent =
  | { kind: 'ready' }
  | { kind: 'failed'; reason: string }
  | { kind: 'add'; text: string }
  | { kind: 'toggle'; id: number }
  | { kind: 'edit_text'; id: number; text: string }
  | { kind: 'reorder'; id: number; after_id: number | null }
  | { kind: 'set_status'; id: number; state: SetStatusTarget }
  | { kind: 'set_plan_date'; id: number; date: string }
  | { kind: 'assign_agent'; id: number; agent: string }
  | { kind: 'set_category'; id: number; category_id: number | null }
  | { kind: 'open_detail'; id: number }
  | { kind: 'add_height'; px: number };
