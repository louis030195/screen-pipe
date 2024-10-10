import React from 'react';
import { Button } from '@/components/ui/button';

interface OnboardingNavigationProps {
  prevBtnText?: string,
  nextBtnText?: string,
  className?: string,
  isLoading?: boolean;
  handlePrevSlide: () => void;
  handleNextSlide: () => void;
}

const OnboardingNavigation: React.FC<OnboardingNavigationProps> = ({ 
  nextBtnText = "",
  prevBtnText = "",
  className = "",
  isLoading,
  handlePrevSlide,
  handleNextSlide,
}) => {
  return (
    <div className={` flex justify-between mx-8 ${className}`}>
      <Button 
        className="w-fit min-w-32 disabled:!cursor-not-allowed disabled:!pointer-events-auto"
        variant={"outline"}
        onClick={handlePrevSlide}
        disabled={isLoading}
      >
        {prevBtnText}
      </Button>
      <Button 
        className="w-fit min-w-32 disabled:!cursor-not-allowed disabled:!pointer-events-auto" 
        onClick={handleNextSlide}
        disabled={isLoading}
      >
        {nextBtnText}
      </Button>
    </div>
  );
} 
export default OnboardingNavigation;

