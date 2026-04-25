/**
 * AREST IconToken set — re-exported from `lucide-react` as named
 * exports so bundlers can tree-shake to exactly the 51 icons listed
 * in readings/ui/design.md.
 *
 * The Lucide registry is canonical; the AREST IconToken names map 1:1
 * to Lucide registry names per the design reading. Some IconTokens use
 * an alias (sort-asc → ArrowUpNarrowWide); we re-export under both the
 * canonical Lucide PascalCase and an AREST-semantic alias so callers
 * can write `<SortAsc />` instead of `<ArrowUpNarrowWide />`.
 *
 * Groupings mirror the Icon Role tags in design.md (file-browser /
 * repl / hateoas / common / status / auth / theme).
 *
 * Lucide v1 normalised icon names (e.g. AlertTriangle → TriangleAlert)
 * but kept the legacy aliases — we use the legacy aliases here so the
 * import names match the Lucide registry names cited in design.md.
 */

// File browser ---------------------------------------------------------------
export {
  File,
  FileText,
  FileCode,
  FileImage,
  FileVideo,
  // No FileAudio in lucide-react v1; alias Music to that semantic role.
  Music as FileAudio,
  FileType,
  // Lucide has no dedicated 'FilePdf' — `FileType` is the closest match
  // and is re-exported under a semantic alias for callers that key by
  // mime-type (application/pdf → FilePdf).
  FileType as FilePdf,
  Folder,
  FolderOpen,
  FolderPlus,
  Upload,
  Download,
} from 'lucide-react'

// REPL -----------------------------------------------------------------------
export {
  Terminal,
  Play,
  Square,
  RotateCcw,
  Copy,
} from 'lucide-react'

// HATEOAS browser ------------------------------------------------------------
export {
  Link,
  ExternalLink,
  ArrowLeft,
  ArrowRight,
  Home,
  Globe,
} from 'lucide-react'

// Common controls ------------------------------------------------------------
export {
  Search,
  X,
  Check,
  Plus,
  Minus,
  Trash,
  Pencil,
  Save,
  Settings,
  Menu,
  MoreHorizontal,
  MoreVertical,
  ChevronRight,
  ChevronLeft,
  ChevronDown,
  ChevronUp,
  Filter,
} from 'lucide-react'

// Sort controls — IconToken names 'sort-asc' / 'sort-desc' map to the
// Lucide canonical names ArrowUpNarrowWide / ArrowDownNarrowWide.
export {
  ArrowUpNarrowWide,
  ArrowUpNarrowWide as SortAsc,
  ArrowDownNarrowWide,
  ArrowDownNarrowWide as SortDesc,
} from 'lucide-react'

// Status / semantic ----------------------------------------------------------
export {
  Info,
  AlertTriangle,
  AlertCircle,
  CheckCircle,
  XCircle,
  Loader,
} from 'lucide-react'

// Auth / user ----------------------------------------------------------------
export {
  User,
  LogIn,
  LogOut,
  Lock,
  Unlock,
} from 'lucide-react'

// Theme switcher -------------------------------------------------------------
export {
  Sun,
  Moon,
  Palette,
} from 'lucide-react'
