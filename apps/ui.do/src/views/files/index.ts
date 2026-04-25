/**
 * File browser barrel — components for the /files surface (#405).
 *
 * Three-column shell (DirectoryTree + FileList + FilePreview) plus
 * the TagFilter chip strip rendered above the list. Mounted at
 *   /files                            — root browser
 *   /files/:directoryId               — directory open
 *   /files/:directoryId/:fileId       — file selected
 * via apps/ui.do/src/App.tsx.
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
