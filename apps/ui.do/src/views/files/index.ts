/**
 * File browser barrel — components for the /files surface (#405, #406).
 *
 * Three-column shell (DirectoryTree + FileList + FilePreview) plus
 * the TagFilter chip strip rendered above the list. Mounted at
 *   /files                            — root browser
 *   /files/:directoryId               — directory open
 *   /files/:directoryId/:fileId       — file selected
 * via apps/ui.do/src/App.tsx.
 *
 * Bulk-ops surface (#406) — selection hook, sticky BulkToolbar, and
 * the four action dialogs (Move / Copy / Delete / Tag / Untag).
 */
export { FileBrowser, type FileBrowserProps } from './FileBrowser'
export { DirectoryTree, type DirectoryTreeProps, type DirectoryRow } from './DirectoryTree'
export {
  FileList,
  pickFileIcon,
  type FileListProps,
  type FileRow,
  type SortField,
  type SortOrder,
  type SortState,
} from './FileList'
export { FilePreview, type FilePreviewProps } from './FilePreview'
export { TagFilter, type TagFilterProps, type TagRow } from './TagFilter'

// Bulk ops (#406)
export { BulkToolbar, type BulkToolbarProps } from './BulkToolbar'
export {
  useFileSelection,
  type FileSelectionApi,
  type FileSelectionState,
  type FileId,
} from './useFileSelection'
export { BaseDialog, type BaseDialogProps } from './dialogs/BaseDialog'
export { MoveDialog, type MoveDialogProps } from './dialogs/MoveDialog'
export { CopyDialog, type CopyDialogProps } from './dialogs/CopyDialog'
export { DeleteConfirmDialog, type DeleteConfirmDialogProps } from './dialogs/DeleteConfirmDialog'
export { TagDialog, type TagDialogProps } from './dialogs/TagDialog'
export { UntagDialog, type UntagDialogProps, type UntagOption } from './dialogs/UntagDialog'
