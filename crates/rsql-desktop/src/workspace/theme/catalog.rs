use gpui_kit::{assets::icon_assets, *};
use std::borrow::Cow;

icon_assets!(
    CatalogIcons,
    [
        Pencil,
        PenLine,
        Table,
        Layers,
        FileCode,
        FileText,
        Copy,
        Trash,
        Zap,
        Database,
        Columns3,
        Key,
        Shield,
        ScrollText,
        Link2,
        GitCompare,
        ArrowLeftRight,
        Code,
        Hash,
        List,
        Settings,
        User,
        Users,
        ShieldCheck,
        ShieldX,
        RefreshCw,
        Check,
        CircleArrowUp,
        Download,
        Activity,
        ChartColumn,
        Lock,
        Gauge,
        Package,
        Terminal,
        Bell,
        BellOff,
        Radio,
        Send,
        X,
        Clock3,
        ZoomIn,
        ZoomOut,
        RotateCcw,
        Columns2,
        GitBranch,
        ListOrdered,
        Play,
        Save,
        Square,
        TextAlignStart,
        Timer,
        Server,
        HardDrive,
        LoaderCircle,
        FileUp,
        Settings2,
        RotateCcwClock,
        Diff,
        Pin,
        Clock,
        CircleCheck,
        CircleX,
        SquarePen,
        PinOff,
        ArrowLeft,
        Link2Off
    ]
);

pub(crate) struct AppAssets;
impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<Cow<'static, [u8]>>> {
        if let Some(icon) = crate::workspace::toast::icon(path) {
            return Ok(Some(Cow::Borrowed(icon)));
        }
        match CatalogIcons.load(path)? {
            Some(icon) => Ok(Some(icon)),
            None => assets::Assets.load(path),
        }
    }
    fn list(&self, path: &str) -> gpui_kit::Result<Vec<SharedString>> {
        let mut paths = assets::Assets.list(path)?;
        paths.extend(CatalogIcons.list(path)?);
        Ok(paths)
    }
}
