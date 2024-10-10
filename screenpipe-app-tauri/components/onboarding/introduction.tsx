import React from "react";
import { DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";

interface OnboardingIntroProps {
  className?: string;
  handleNextSlide: () => void;
}

const OnboardingIntro: React.FC<OnboardingIntroProps> = ({
  className = "",
  handleNextSlide,
}) => (
  <div className={` flex justify-center items-center flex-col ${className}`}>
    <DialogHeader className="px-2">
      <div className="w-full !mt-[-10px] inline-flex justify-center">
        <img
          src="/128x128.png"
          alt="screenpipe-logo"
          width="72"
          height="72"
        />
      </div>
      <DialogTitle className="!mt-[-2px] font-bold text-nowrap text-center text-[24px] flex justify-center">
        hey! we&apos;re excited to have you in the screenpipe community!
      </DialogTitle>
      <p className="text-center !mt-[0px] text-base">
        get ready to discover all the amazing things our product has
        to offer!!
      </p>
    </DialogHeader>
    <video
      width="600px"
      className="mt-2 rounded-md"
      autoPlay
      controls
      preload="true"
    >
      <source src="/onboarding-screenpipe.mp4" type="video/mp4" />
      your browser does not support the video tag.
    </video>
    <Button
      className="mt-5 w-28 ml-auto float-right mr-20"
      onClick={handleNextSlide}
    >
      get started
    </Button>
  </div>
);

export default OnboardingIntro;

