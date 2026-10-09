import Image from "next/image";
import litellmMonogram from "../../../public/assets/logos/litellm_monogram.svg";
import lensSymbol from "../../assets/lens-symbol.svg";
import { cn } from "../../lib/cva.config";

export function LensBrand({ className }: { className?: string }) {
  return (
    <span className={cn("inline-flex shrink-0 items-center gap-2", className)}>
      <span className="inline-flex items-center gap-1.5">
        <Image
          src={litellmMonogram}
          alt="LiteLLM"
          width={28}
          height={28}
          className="size-7 brightness-0 dark:invert"
          loading="eager"
          unoptimized
        />
        <Image src={lensSymbol} alt="" width={30} height={30} className="size-[30px]" loading="eager" unoptimized />
      </span>
      <span className="text-xl leading-none font-semibold tracking-tight text-sidebar-foreground">Lens</span>
    </span>
  );
}
