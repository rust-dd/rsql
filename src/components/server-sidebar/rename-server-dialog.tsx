import React from "react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

export function RenameServerDialog({
  open,
  onOpenChange,
  address,
  currentAlias,
  onRename,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  address: string;
  currentAlias: string;
  onRename: (alias: string) => Promise<void>;
}) {
  const [alias, setAlias] = React.useState("");

  React.useEffect(() => {
    if (open) setAlias(currentAlias);
  }, [open, currentAlias]);

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    void onRename(alias);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[400px]">
        <DialogHeader>
          <DialogTitle className="font-mono">Rename Server</DialogTitle>
          <DialogDescription>
            Name shown for{" "}
            <span className="font-mono font-semibold text-foreground">{address}</span>. Leave it
            empty to go back to the default label.
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit} className="space-y-3 mt-2">
          <div className="space-y-1">
            <Label htmlFor="serverAlias" className="font-mono text-xs">
              Server Name
            </Label>
            <Input
              id="serverAlias"
              value={alias}
              onChange={(e) => setAlias(e.target.value)}
              placeholder={address}
              autoFocus
              className="font-mono text-sm h-8"
            />
          </div>
          <div className="flex justify-end gap-2 pt-2">
            <Button
              type="button"
              variant="ghost"
              className="text-xs"
              onClick={() => onOpenChange(false)}
            >
              Cancel
            </Button>
            <Button type="submit" variant="gradient" className="text-xs">
              Save
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}
