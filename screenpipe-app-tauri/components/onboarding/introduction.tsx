import React from "react";
import { DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";

interface OnboardingIntroProps {
  handleNextSlide: () => void;
}

const OnboardingIntro: React.FC<OnboardingIntroProps> = ({
  handleNextSlide,
}) => (
  <div className="flex justify-center items-center flex-col">
    <DialogHeader className="mt-1 px-2">
      <div className="w-full inline-flex justify-center">
        <img
          src="/128x128.png"
          alt="screenpipe-logo"
          width="72"
          height="72"
        />
      </div>
      <DialogTitle className="!mt-[-5px] text-center text-[23px] text-balance flex justify-center">
        Heya! We're stoked to have you as part of Screenpipe Community!
      </DialogTitle>
      <p className="text-center !mt-[0px] text-base">
        Get ready to discover all the amazing things our product has
        to offer!!
      </p>
    </DialogHeader>
    <video
      width="85%"
      className="mt-2 rounded-md"
      autoPlay
      controls
      preload="true"
    >
      <source src="/onboarding-screenpipe.mp4" type="video/mp4" />
      Your browser does not support the video tag.
    </video>
    <Button
      className="mt-5 w-28 ml-auto float-right mr-20"
      onClick={handleNextSlide}
    >
      Get Started
    </Button>
  </div>
);

export default OnboardingIntro;

