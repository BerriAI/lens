import Image from "next/image";
import litellmMonogram from "../../../public/assets/logos/litellm_monogram.svg";
import lensSymbol from "../../assets/lens-symbol.svg";
import { cn } from "../../lib/cva.config";

export function LensBrand({
  className,
  showLiteLLM = true,
}: {
  className?: string;
  showLiteLLM?: boolean;
}) {
  return (
    <span className={cn("inline-flex shrink-0 items-center gap-2", className)}>
      <span className="inline-flex items-center gap-1.5">
        {showLiteLLM && (
          <span
            role="img"
            aria-label="LiteLLM"
            className="lens-brand-monogram size-7"
            style={{ maskImage: `url(${litellmMonogram.src})` }}
          />
        )}
        <Image
          src={lensSymbol}
          alt=""
          width={showLiteLLM ? 30 : 24}
          height={showLiteLLM ? 30 : 24}
          className={showLiteLLM ? "size-[30px]" : "size-6"}
          loading="eager"
          unoptimized
        />
      </span>
      <span className="lens-brand-name text-xl leading-none font-semibold tracking-tight">
        Lens
      </span>
    </span>
  );
}
