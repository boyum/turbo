import path from "path";
import chalk from "chalk";
import childProcess from "child_process";
import { setupTestFixtures, spyConsole } from "@turbo/test-utils";
import { create } from "../src/commands/create";
import type { CreateCommandArgument } from "../src/commands/create/types";
import { logger } from "@turbo/utils";
import type { PackageManager } from "@turbo/workspaces";

// imports for mocks
import * as turboWorkspaces from "@turbo/workspaces";
import * as turboUtils from "@turbo/utils";
import { getWorkspaceDetailsMockReturnValue } from "./test-utils";

jest.mock("@turbo/workspaces", () => ({
  __esModule: true,
  ...jest.requireActual("@turbo/workspaces"),
}));

describe("create-turbo", () => {
  const { useFixture } = setupTestFixtures({
    directory: path.join(__dirname, "../"),
    options: { emptyFixture: true },
  });

  const mockConsole = spyConsole();

  test.each<{ packageManager: PackageManager }>([
    { packageManager: "yarn" },
    { packageManager: "npm" },
    { packageManager: "pnpm" },
  ])(
    "outputs expected console messages when using $packageManager",
    async ({ packageManager }) => {
      const { root } = useFixture({ fixture: `create-turbo` });

      const availableScripts = ["build", "test", "dev"];

      const mockAvailablePackageManagers = jest
        .spyOn(turboUtils, "getAvailablePackageManagers")
        .mockResolvedValue({
          npm: {
            available: true,
            version: "8.19.2",
          },
          yarn: {
            available: true,
            version: "1.22.10",
          },
          pnpm: {
            available: true,
            version: "7.22.2",
          },
        });

      const mockCreateProject = jest
        .spyOn(turboUtils, "createProject")
        .mockResolvedValue({
          cdPath: "",
          hasPackageJson: true,
          availableScripts,
        });

      const mockGetWorkspaceDetails = jest
        .spyOn(turboWorkspaces, "getWorkspaceDetails")
        .mockResolvedValue(
          getWorkspaceDetailsMockReturnValue({
            root,
            packageManager,
          })
        );

      const mockExecSync = jest
        .spyOn(childProcess, "execSync")
        .mockImplementation(() => {
          return "success";
        });

      await create(
        root as CreateCommandArgument,
        packageManager as CreateCommandArgument,
        {
          skipInstall: true,
          example: "default",
        }
      );

      const expected = `${chalk.bold(
        logger.turboGradient(">>> Success!")
      )} Created a new Turborepo at "${path.relative(process.cwd(), root)}".`;

      expect(mockConsole.log).toHaveBeenCalledWith(expected);
      expect(mockConsole.log).toHaveBeenCalledWith(
        "Inside that directory, you can run several commands:"
      );

      availableScripts.forEach((script) => {
        expect(mockConsole.log).toHaveBeenCalledWith(
          chalk.cyan(`  ${packageManager} run ${script}`)
        );
      });

      mockAvailablePackageManagers.mockRestore();
      mockCreateProject.mockRestore();
      mockGetWorkspaceDetails.mockRestore();
      mockExecSync.mockRestore();
    }
  );
});