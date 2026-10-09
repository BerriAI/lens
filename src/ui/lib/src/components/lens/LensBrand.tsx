import Image from "next/image";
import litellmMonogram from "../../../public/assets/logos/litellm_monogram.svg";
import lensSymbol from "../../assets/lens-symbol.svg";
import { cn } from "../../lib/cva.config";

export function LensBrand({ className }: { className?: string }) {
  return (
    <span className={cn("inline-flex shrink-0 items-center gap-2", className)}>
      <span className="inline-flex items-center gap-1.5">
        <span
          role="img"
          aria-label="LiteLLM"
          className="lens-brand-monogram size-7"
          style={{ maskImage: `url(${litellmMonogram.src})` }}
        />
        <Image src={lensSymbol} alt="" width={30} height={30} className="size-[30px]" loading="eager" unoptimized />
      </span>
      <span className="lens-brand-name text-xl leading-none font-semibold tracking-tight">Lens</span>
    </span>
  );
}
