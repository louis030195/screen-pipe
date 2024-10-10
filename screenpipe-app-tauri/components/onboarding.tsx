import localforage from "localforage";
import React, { useState, useEffect } from "react";
import { useToast } from "@/components/ui/use-toast";
import OnboardingPipes from "@/components/onboarding/pipes";
import { Dialog, DialogContent } from "@/components/ui/dialog";
import OnboardingStatus from "@/components/onboarding/status";
import OnboardingIntro from "@/components/onboarding/introduction";
import OnboardingAPISetup from "@/components/onboarding/api-setup";
import OnboardingPersonalize from "@/components/onboarding/personalize";
import OnboardingDevOrNonDev from "@/components/onboarding/dev-or-non-dev";
import OnboardingDevConfig from "@/components/onboarding/dev-configuration";
import OnboardingSelection from "@/components/onboarding/usecases-selection";
import OnboardingInstructions from "@/components/onboarding/explain-instructions";
import OnboardingExperimentalFeatures from "@/components/onboarding/features-experimental";

const setFirstTimeUserFlag = async () => {
  await localforage.setItem("isFirstTimeUser", false);
};

type SlideKey =
  | "intro"
  | "status"
  | "selection"
  | "personalize"
  | "apiSetup"
  | "devOrNonDev"
  | "devConfig"
  | "pipes"
  | "instructions"
  | "experimentalFeatures";

const slideFlow: Record<
  SlideKey, {
    next: (
      selectedOptions?: string[],
      selectedPreference?: string | null,
      selectedPersonalization?: string | null,
    ) => SlideKey | null;
    prev: (
      selectedOptions?: string[],
      selectedPreference?: string | null,
      selectedPersonalization?: string | null,
    ) => SlideKey | null;
  }
> = {
  intro: {                                                                                        // introduction video of screenpipe
    next: () => "status",
    prev: () => null,
  },
  status: {                                                                                       // status of screenpipe (blockage or not) 
    next: () => "selection", 
    prev: () => "intro", 
  },                                             
  selection: {                                                                                    // selection (four options)
    next: (selectedOptions) => {
      if (!Array.isArray(selectedOptions) || selectedOptions.length === 0) return null;
      if (selectedOptions.includes("personalUse")) return "personalize";
      if (selectedOptions.includes("professionalUse")) return "apiSetup";
      if (selectedOptions.includes("developmentlUse")) return "devOrNonDev";
      if (selectedOptions.includes("otherUse")) return "devOrNonDev";
      return "instructions";
    },
    prev: () => "status",
  },
  personalize: {                                                                                    // personalize (with ai or without ai)
    next: (_, __, selectedPersonalization) => 
      selectedPersonalization === "withAI" ? "apiSetup" : "instructions",
    prev: () => "selection",
  },
  apiSetup: {                                                                                       // api setup & validation
    next: () => "instructions",
    prev: (selectedOptions) => {
      if (selectedOptions?.length === 1 && selectedOptions?.includes("professionalUse")) {
        return "selection";
      } 
      return "personalize"
    },
  },
  devOrNonDev: {                                                                                    // dev or no dev
    next: (_, selectedPreference) => 
      selectedPreference === "devMode" ? "devConfig" : "personalize",
    prev: () => "selection",
  },
  devConfig: {                                                                                      // dev configuration
    next: () => "pipes", 
    prev: () => "devOrNonDev"
  },
  pipes: {                                                                                          // explain about pipes to dev
    next: () => "instructions", 
    prev: () => "devConfig",
  },
  instructions: {                                                                                   // instructions for every type of user
    next: () => "experimentalFeatures", 
    prev: (selectedOptions, selectedPreference, selectedPersonalization) => {
      if (selectedPreference === "devMode") return "pipes";
      if (selectedOptions?.includes("personalUse")) return "personalize";
      if (selectedOptions?.includes("professionalUse")) return "apiSetup";
      if (selectedOptions?.includes("developmentlUse") && selectedPreference === "nonDevMode" && selectedPersonalization === "withoutAI") return "personalize";
      if (selectedOptions?.includes("developmentlUse") || selectedOptions?.includes("otherUse") && selectedPersonalization === "withAI") return "apiSetup";
      return "devOrNonDev";
    }
  },
  experimentalFeatures: {                                                                           // experimental features
    next: () => null, 
    prev: () => "instructions",
  },
};

const Onboarding: React.FC = () => {
  const { toast } = useToast();
  const [isOpen, setIsOpen] = useState(false);
  const [currentSlide, setCurrentSlide] = useState<SlideKey>("intro");
  const [selectedOptions, setSelectedOptions] = useState<string[]>([]);                             // use case selection (four options)
  const [selectedPersonalization, setSelectedPersonalization] = useState<string | null >(null);     // with ai or without ai
  const [selectedPreference, setSelectedPreference] = useState<string | null >(null);               // dev or non dev
  const [error, setError] = useState<string | null>(null);
  const [isVisible, setIsVisible] = useState(false);

  useEffect(() => {
    setIsVisible(true);
  }, [currentSlide]);

  useEffect(() => {
    if (isOpen) {
      const hideCloseButton = () => {
        const closeButton = document.querySelector(".lucide-x");
        if (closeButton) {
          (closeButton as HTMLElement).classList.add("hidden");
        }
      };
      setTimeout(hideCloseButton, 100);
    }
  }, [isOpen]);

  useEffect(() => {
    const checkFirstTimeUser = async () => {
      const isFirstTime = await localforage.getItem<boolean>("isFirstTimeUser");
      if (isFirstTime === null) {
        setIsOpen(true);
      }
    };
    checkFirstTimeUser();
  }, []);

  useEffect(() => {
    if (error) {
      toast({
        title: "error",
        description: error,
        variant: "destructive",
      });
    }
  }, [error, toast]);

  const handleNextSlide = () => {
    const nextSlide = slideFlow[currentSlide].next(
      selectedOptions,
      selectedPreference,
      selectedPersonalization,
    );
    if (currentSlide === "selection" && (!selectedOptions || selectedOptions.length === 0)) {
      setError("please select at least one option before proceeding!");
      return;
    }
    if (currentSlide === "personalize" && !selectedPersonalization) {
      setError("please choose a personalization option!");
      return;
    }
    if (currentSlide === "devOrNonDev" && !selectedPreference) {
      setError("please choose a preference option!");
      return;
    }
    if (nextSlide) {
      setIsVisible(false)
      setTimeout(() => {
        setCurrentSlide(nextSlide);
        setError(null);
      }, 300);
    } else {
      setError("Please validate selection")
    } 
  };

  const handlePrevSlide = () => {
    setIsVisible(false);
    setTimeout(() => {
      const prevSlide = slideFlow[currentSlide].prev(
        selectedOptions,
        selectedPreference,
        selectedPersonalization,
      );
      if (prevSlide) {
        setError(null);
        setCurrentSlide(prevSlide);
      }
    }, 300);
  };

  const handleOptionClick = (option: string) => {
    setSelectedOptions((prevOptions) =>
      prevOptions.includes(option)
        ? prevOptions.filter((opt) => opt !== option)
        : [...prevOptions, option]
    );
    setError(null);
  };

  const handleDialogClose = (isOpen: boolean) => {
    if (!isOpen && currentSlide) {
      return;
    }
    setIsOpen(isOpen);
  };

  const handleEnd = async () => {
    setIsOpen(false);
    await setFirstTimeUserFlag();
    window.location.reload();
  };

  return (
    <Dialog open={isOpen} onOpenChange={handleDialogClose}>
      <DialogContent className="max-w-3xl h-[580px] max-h-[80vh]">
        <div className="relative w-full h-full overflow-hidden">
          {currentSlide === "intro" && (
            <OnboardingIntro 
              className={`transition-opacity duration-300 
              ${isVisible ? 'opacity-100 ease-out' : 'opacity-0 ease-in'}`}
              handleNextSlide={handleNextSlide} 
            />
          )}
          {currentSlide === "status" && (
            <OnboardingStatus
              className={`transition-opacity duration-300 
              ${isVisible ? 'opacity-100 ease-out' : 'opacity-0 ease-in'}`}
              handlePrevSlide={handlePrevSlide}
              handleNextSlide={handleNextSlide} 
            />
          )}
          {currentSlide === "selection" && (
            <OnboardingSelection
              className={`transition-opacity duration-300 ease-in-out 
              ${isVisible ? 'opacity-100 ease-out' : 'opacity-0 ease-in'}`}
              handleOptionClick={handleOptionClick}
              selectedOptions={selectedOptions}
              handlePrevSlide={handlePrevSlide}
              handleNextSlide={handleNextSlide}
            />
          )}
          {currentSlide === "personalize" && (
            <OnboardingPersonalize
              className={`transition-opacity duration-300 ease-in-out 
              ${isVisible ? 'opacity-100 ease-out' : 'opacity-0 ease-in'}`}
              handleOptionClick={setSelectedPersonalization}
              selectedPersonalization={selectedPersonalization}
              handlePrevSlide={handlePrevSlide}
              handleNextSlide={handleNextSlide}
            />
          )}
          {currentSlide === "apiSetup" && (
            <OnboardingAPISetup
              className={`transition-opacity duration-300 ease-in-out 
              ${isVisible ? 'opacity-100 ease-out' : 'opacity-0 ease-in'}`}
              handleNextSlide={handleNextSlide}
              handlePrevSlide={handlePrevSlide}
            />
          )}
          {currentSlide === "devOrNonDev" && (
            <OnboardingDevOrNonDev
              className={`transition-opacity duration-300 ease-in-out 
              ${isVisible ? 'opacity-100 ease-out' : 'opacity-0 ease-in'}`}
              handleOptionClick={setSelectedPreference}
              selectedPreference={selectedPreference}
              handlePrevSlide={handlePrevSlide}
              handleNextSlide={handleNextSlide}
            />
          )}
          {currentSlide === "devConfig" && (
            <OnboardingDevConfig
              className={`transition-opacity duration-300 ease-in-out 
              ${isVisible ? 'opacity-100 ease-out' : 'opacity-0 ease-in'}`}
              handleNextSlide={handleNextSlide}
              handlePrevSlide={handlePrevSlide}
            />
          )}
          {currentSlide === "pipes" && (
            <OnboardingPipes
              className={`transition-opacity duration-300 ease-in-out 
              ${isVisible ? 'opacity-100 ease-out' : 'opacity-0 ease-in'}`}
              handleNextSlide={handleNextSlide}
              handlePrevSlide={handlePrevSlide}
            />
          )}
          {currentSlide === "instructions" && (
            <OnboardingInstructions
              className={`transition-opacity duration-300 ease-in-out 
              ${isVisible ? 'opacity-100 ease-out' : 'opacity-0 ease-in'}`}
              handleNextSlide={handleNextSlide}
              handlePrevSlide={handlePrevSlide}
            />
          )}
          {currentSlide === "experimentalFeatures" && (
            <OnboardingExperimentalFeatures
              className={`transition-opacity duration-300 
              ${isVisible ? 'opacity-100' : 'opacity-0'}`}
              handleNextSlide={handleEnd}
              handlePrevSlide={handlePrevSlide}
            />
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
};

export default Onboarding;

